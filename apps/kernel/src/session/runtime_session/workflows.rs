use super::*;

impl RuntimeSession {
    /// A workflow run can lose its provider prompt while the kernel remains alive, for
    /// example if the provider process exits between prompt acknowledgement and the
    /// completion transition.  Such a run still looks active to session arbitration and
    /// blocks every later workflow queue head.  Restart reconciliation handles this case
    /// during boot; this live variant is used by queue admission as a safety net.
    pub fn reconcile_live_orphaned_workflow_runs(
        &mut self,
        now_ms: u64,
        grace_period_ms: u64,
    ) -> usize {
        let durable_workflow_prompt_targets = self
            .prompt_runtime
            .prompt_states()
            .values()
            .flat_map(|state| {
                state
                    .active_prompt()
                    .into_iter()
                    .chain(state.queued_prompts())
            })
            .filter_map(|prompt| {
                Some((
                    prompt.workflow_run_id()?.to_string(),
                    prompt.workflow_node_run_id()?.to_string(),
                ))
            })
            .collect::<BTreeSet<_>>();
        let mut orphaned_workflow_run_ids = Vec::new();
        let settling_workflow_run_ids = self
            .settling_workflow_run_counts
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();

        for workflow_run in &mut self.workflow_runs {
            if workflow_run.status() != WorkflowRunStatus::Running
                || durable_workflow_prompt_targets
                    .iter()
                    .any(|(run_id, _)| run_id == workflow_run.id())
                || settling_workflow_run_ids.contains(workflow_run.id())
            {
                continue;
            }
            if workflow_run
                .node_runs()
                .iter()
                .any(|node| node.status() == WorkflowNodeRunStatus::BlockedOnWorkspaceClaim)
            {
                continue;
            }
            let orphaned_node_run_ids = workflow_run
                .node_runs()
                .iter()
                .filter(|node| {
                    !matches!(
                        node.status(),
                        WorkflowNodeRunStatus::Completed
                            | WorkflowNodeRunStatus::Failed
                            | WorkflowNodeRunStatus::Stopped
                    )
                })
                .map(|node| node.id().to_string())
                .collect::<Vec<_>>();
            if orphaned_node_run_ids.is_empty()
                || orphaned_node_run_ids.iter().any(|node_run_id| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node| node.id() == node_run_id)
                        .and_then(|node| node.started_at_ms().or(Some(node.created_at_ms())))
                        .is_some_and(|started_at_ms| {
                            now_ms.saturating_sub(started_at_ms) < grace_period_ms
                        })
                })
            {
                continue;
            }

            for node_run in workflow_run.node_runs_mut() {
                if !orphaned_node_run_ids.iter().any(|id| id == node_run.id()) {
                    continue;
                }
                node_run.set_status(WorkflowNodeRunStatus::Stopped);
                if let Some(envelope) = node_run.turn_envelope_mut() {
                    envelope.mark_cancelled();
                }
            }
            let failure_source_node_run_id = workflow_run
                .active_node_run_id()
                .map(str::to_string)
                .or_else(|| orphaned_node_run_ids.first().cloned())
                .unwrap_or_else(|| workflow_run.id().to_string());
            workflow_run.clear_active_node_run();
            workflow_run.add_failure_event(WorkflowFailureEvent::new(
                WorkflowFailureKind::RunStopped,
                failure_source_node_run_id,
                Vec::new(),
                "non-terminal workflow run had no live active or queued provider prompt",
            ));
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            orphaned_workflow_run_ids.push(workflow_run.id().to_string());
        }

        for workflow_run_id in &orphaned_workflow_run_ids {
            self.prompt_runtime.remove_queued_prompts_by_workflow_run(
                workflow_run_id,
                self.focused_agent_id.as_deref(),
            );
        }
        orphaned_workflow_run_ids.len()
    }

    pub fn create_workflow(&mut self, workflow: WorkflowDefinition) -> WorkflowDefinition {
        let workflow_id = workflow.id().to_string();
        self.workflows.push(workflow.clone());
        self.ensure_default_workflow_prompt_queue(&workflow_id);
        workflow
    }

    /// Drop queued workflow records that cannot be dispatched anymore because their
    /// workflow, endpoint, or queue was removed by an older runtime.
    pub fn reconcile_workflow_queue_ownership(&mut self) -> usize {
        let valid_workflows = self
            .workflows
            .iter()
            .map(|workflow| workflow.id().to_string())
            .collect::<BTreeSet<_>>();
        let valid_endpoints = self
            .workflows
            .iter()
            .flat_map(|workflow| {
                workflow
                    .endpoints()
                    .iter()
                    .map(move |endpoint| (workflow.id().to_string(), endpoint.id().to_string()))
            })
            .collect::<BTreeSet<_>>();
        let valid_queues = self
            .workflow_prompt_queues
            .iter()
            .flat_map(|queue| {
                [queue.id(), queue.alias()]
                    .into_iter()
                    .map(move |queue_ref| (queue.workflow_id().to_string(), queue_ref.to_string()))
            })
            .collect::<BTreeSet<_>>();
        let before = self.workflow_queued_prompts.len();
        self.workflow_queued_prompts.retain(|prompt| {
            valid_workflows.contains(prompt.workflow_id())
                && valid_endpoints.contains(&(
                    prompt.workflow_id().to_string(),
                    prompt.endpoint_id().to_string(),
                ))
                && valid_queues.contains(&(
                    prompt.workflow_id().to_string(),
                    prompt.queue_id().to_string(),
                ))
        });
        before.saturating_sub(self.workflow_queued_prompts.len())
    }

    pub(crate) fn replace_publication_runtime_workflows(
        &mut self,
        workflows: Vec<WorkflowDefinition>,
        workflow_prompt_queues: Vec<WorkflowPromptQueueDefinition>,
        workflow_schedules: Vec<WorkflowScheduleDefinition>,
    ) {
        self.workflows = workflows;
        self.workflow_prompt_queues = workflow_prompt_queues;
        self.workflow_schedules = workflow_schedules;
        let workflow_ids = self
            .workflows
            .iter()
            .map(|workflow| workflow.id().to_string())
            .collect::<Vec<_>>();
        for workflow_id in workflow_ids {
            self.ensure_default_workflow_prompt_queue(&workflow_id);
        }
        // Materializing a publication replaces the runtime-owned workflow graph.
        // Drop queue records that point at the previous graph immediately. Waiting
        // for restart reconciliation leaves them visible in the session-wide queue
        // inventory and makes them look like they belong to the replacement workflow.
        self.reconcile_workflow_queue_ownership();
    }

    pub fn remove_workflow(&mut self, workflow_id: &str) -> Option<WorkflowDefinition> {
        let index = self
            .workflows
            .iter()
            .position(|workflow| workflow.id() == workflow_id)?;
        let removed = self.workflows.remove(index);
        self.workflow_prompt_queues
            .retain(|queue| queue.workflow_id() != workflow_id);
        self.workflow_queued_prompts
            .retain(|prompt| prompt.workflow_id() != workflow_id);
        self.workflow_schedules
            .retain(|schedule| schedule.workflow_id() != workflow_id);
        self.workflow_consoles
            .retain(|console| console.workflow_id() != workflow_id);
        for instance in self
            .workflow_runtime_instances
            .iter_mut()
            .filter(|instance| instance.workflow_id() == workflow_id)
        {
            instance.mark_stale();
        }
        Some(removed)
    }

    pub fn workflow(&self, workflow_id: &str) -> Option<&WorkflowDefinition> {
        self.workflows
            .iter()
            .find(|workflow| workflow.id() == workflow_id)
    }

    pub fn workflow_mut(&mut self, workflow_id: &str) -> Option<&mut WorkflowDefinition> {
        self.workflows
            .iter_mut()
            .find(|workflow| workflow.id() == workflow_id)
    }

    pub fn create_workflow_publication(
        &mut self,
        publication: WorkflowPublicationDefinition,
        source_snapshot: Option<WorkflowPublicationSnapshot>,
    ) -> WorkflowPublicationDefinition {
        if let Some(source_snapshot) = source_snapshot {
            self.workflow_publication_state
                .workflow_publication_snapshots
                .insert(publication.id().to_string(), source_snapshot);
        }
        self.workflow_publication_state
            .workflow_publications
            .push(publication.clone());
        publication
    }

    pub fn workflow_publication(
        &self,
        publication_id: &str,
    ) -> Option<&WorkflowPublicationDefinition> {
        self.workflow_publication_state
            .workflow_publications
            .iter()
            .find(|publication| publication.id() == publication_id)
    }

    pub fn workflow_publication_mut(
        &mut self,
        publication_id: &str,
    ) -> Option<&mut WorkflowPublicationDefinition> {
        self.workflow_publication_state
            .workflow_publications
            .iter_mut()
            .find(|publication| publication.id() == publication_id)
    }

    pub(crate) fn replace_publication_runtime_configuration(
        &mut self,
        publication_id: &str,
        snapshot: WorkflowPublicationSnapshot,
    ) -> Result<(), String> {
        let digest = snapshot.digest().map_err(|error| error.to_string())?;
        let publication = self
            .workflow_publication_mut(publication_id)
            .ok_or_else(|| "publication runtime is missing".to_string())?;
        publication.set_runtime_snapshot_digest(digest)?;
        self.workflow_publication_state
            .workflow_publication_snapshots
            .insert(publication_id.to_string(), snapshot);
        Ok(())
    }

    pub fn create_workflow_event_binding(
        &mut self,
        binding: WorkflowEventBinding,
    ) -> WorkflowEventBinding {
        self.workflow_publication_state
            .workflow_event_bindings
            .push(binding.clone());
        binding
    }

    pub fn workflow_event_binding(&self, binding_id: &str) -> Option<&WorkflowEventBinding> {
        self.workflow_publication_state
            .workflow_event_bindings
            .iter()
            .find(|binding| binding.id == binding_id)
    }

    pub fn workflow_event_binding_mut(
        &mut self,
        binding_id: &str,
    ) -> Option<&mut WorkflowEventBinding> {
        self.workflow_publication_state
            .workflow_event_bindings
            .iter_mut()
            .find(|binding| binding.id == binding_id)
    }

    pub fn remove_workflow_event_binding(
        &mut self,
        binding_id: &str,
    ) -> Option<WorkflowEventBinding> {
        let index = self
            .workflow_publication_state
            .workflow_event_bindings
            .iter()
            .position(|binding| binding.id == binding_id)?;
        Some(
            self.workflow_publication_state
                .workflow_event_bindings
                .remove(index),
        )
    }

    pub fn record_workflow_event_delivery_receipt(
        &mut self,
        receipt: WorkflowEventDeliveryReceipt,
    ) {
        self.workflow_publication_state
            .workflow_event_delivery_receipts
            .insert(receipt.delivery_id.clone(), receipt);
    }

    pub fn prune_expired_workflow_event_delivery_receipts(&mut self, now_ms: u64) {
        self.workflow_publication_state
            .workflow_event_delivery_receipts
            .retain(|_, receipt| receipt.expires_at_ms > now_ms);
    }

    pub fn create_workflow_run(&mut self, workflow_run: WorkflowRun) -> WorkflowRun {
        self.workflow_runs.push(workflow_run.clone());
        workflow_run
    }

    pub fn workflow_runtime_instances(&self) -> &[WorkflowEndpointRuntimeInstance] {
        &self.workflow_runtime_instances
    }

    pub fn workflow_runtime_instance(
        &self,
        instance_id: &str,
    ) -> Option<&WorkflowEndpointRuntimeInstance> {
        self.workflow_runtime_instances
            .iter()
            .find(|instance| instance.id() == instance_id)
    }

    pub fn add_workflow_runtime_instance(
        &mut self,
        instance: WorkflowEndpointRuntimeInstance,
    ) -> WorkflowEndpointRuntimeInstance {
        self.workflow_runtime_instances.push(instance.clone());
        instance
    }

    pub fn remove_workflow_runtime_instance(
        &mut self,
        instance_id: &str,
    ) -> Option<WorkflowEndpointRuntimeInstance> {
        let index = self
            .workflow_runtime_instances
            .iter()
            .position(|instance| instance.id() == instance_id)?;
        Some(self.workflow_runtime_instances.remove(index))
    }

    pub fn idle_workflow_runtime_instance(
        &self,
        workflow_id: &str,
        endpoint_id: &str,
        workflow_revision: u64,
    ) -> Option<&WorkflowEndpointRuntimeInstance> {
        self.workflow_runtime_instances
            .iter()
            .filter(|instance| {
                instance.workflow_id() == workflow_id
                    && instance.endpoint_id() == endpoint_id
                    && instance.workflow_revision() == workflow_revision
                    && instance.status() == WorkflowEndpointRuntimeInstanceStatus::Idle
            })
            .min_by_key(|instance| instance.ordinal())
    }

    pub fn current_workflow_runtime_instance_count(
        &self,
        workflow_id: &str,
        endpoint_id: &str,
        workflow_revision: u64,
    ) -> usize {
        self.workflow_runtime_instances
            .iter()
            .filter(|instance| {
                instance.workflow_id() == workflow_id
                    && instance.endpoint_id() == endpoint_id
                    && instance.workflow_revision() == workflow_revision
                    && instance.status() != WorkflowEndpointRuntimeInstanceStatus::Stale
            })
            .count()
    }

    pub fn next_workflow_runtime_instance_ordinal(
        &self,
        workflow_id: &str,
        endpoint_id: &str,
    ) -> u16 {
        self.workflow_runtime_instances
            .iter()
            .filter(|instance| {
                instance.workflow_id() == workflow_id && instance.endpoint_id() == endpoint_id
            })
            .map(WorkflowEndpointRuntimeInstance::ordinal)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn claim_workflow_runtime_instance(
        &mut self,
        instance_id: &str,
        workflow_run_id: &str,
    ) -> Option<WorkflowEndpointRuntimeInstance> {
        let instance = self
            .workflow_runtime_instances
            .iter_mut()
            .find(|instance| instance.id() == instance_id)?;
        instance
            .claim(workflow_run_id.to_string())
            .then(|| instance.clone())
    }

    pub fn release_workflow_runtime_instance_for_run(
        &mut self,
        workflow_run_id: &str,
    ) -> Option<WorkflowEndpointRuntimeInstance> {
        let instance = self
            .workflow_runtime_instances
            .iter_mut()
            .find(|instance| instance.active_run_id() == Some(workflow_run_id))?;
        instance.release(workflow_run_id).then(|| instance.clone())
    }

    pub fn mark_workflow_runtime_instance_stale(
        &mut self,
        instance_id: &str,
    ) -> Option<WorkflowEndpointRuntimeInstance> {
        let instance = self
            .workflow_runtime_instances
            .iter_mut()
            .find(|instance| instance.id() == instance_id)?;
        instance.mark_stale();
        Some(instance.clone())
    }

    pub(crate) fn retarget_workflow_runtime_instances_revision(
        &mut self,
        workflow_id: &str,
        workflow_revision: u64,
    ) {
        for instance in self
            .workflow_runtime_instances
            .iter_mut()
            .filter(|instance| instance.workflow_id() == workflow_id)
        {
            instance.retarget_workflow_revision(workflow_revision);
        }
    }

    pub(crate) fn invalidate_workflow_runtime_instances_for_agent_change(
        &mut self,
        agent_id: &str,
    ) -> Vec<String> {
        let mut affected = BTreeMap::new();
        for workflow in &mut self.workflows {
            if workflow
                .nodes()
                .iter()
                .any(|node| node.agent_id() == agent_id)
            {
                workflow.bump_revision();
                affected.insert(workflow.id().to_string(), workflow.revision());
            }
        }
        if affected.is_empty() {
            return Vec::new();
        }

        // The primary instance uses the edited source agents directly and can stay
        // reusable at the new revision. Copies contain materialized agent snapshots;
        // leave them on the old revision so reconciliation retires idle copies now
        // and busy copies as soon as their current run finishes.
        for instance in &mut self.workflow_runtime_instances {
            let Some(revision) = affected.get(instance.workflow_id()).copied() else {
                continue;
            };
            if instance.primary() {
                instance.retarget_workflow_revision(revision);
            }
        }
        self.reconcile_workflow_runtime_instances();
        affected.into_keys().collect()
    }

    pub fn reconcile_workflow_runtime_instances(&mut self) {
        let active_runs = self
            .workflow_runs
            .iter()
            .filter(|run| !run.status().is_terminal())
            .filter_map(|run| Some((run.runtime_instance_id()?.to_string(), run.id().to_string())))
            .collect::<BTreeMap<_, _>>();
        let workflow_revisions = self
            .workflows
            .iter()
            .map(|workflow| (workflow.id().to_string(), workflow.revision()))
            .collect::<BTreeMap<_, _>>();
        for instance in &mut self.workflow_runtime_instances {
            let active_run_id = active_runs.get(instance.id());
            let revision_is_current = workflow_revisions.get(instance.workflow_id()).copied()
                == Some(instance.workflow_revision());
            if let Some(run_id) = active_run_id {
                if instance.status() == WorkflowEndpointRuntimeInstanceStatus::Idle {
                    instance.claim(run_id.clone());
                }
            } else if let Some(run_id) = instance.active_run_id().map(str::to_string) {
                instance.release(&run_id);
            }
            if !revision_is_current && active_run_id.is_none() {
                instance.mark_stale();
            }
        }
    }

    pub fn cleanup_ready_workflow_runtime_instances(
        &mut self,
    ) -> Vec<WorkflowEndpointRuntimeInstance> {
        self.reconcile_workflow_runtime_instances();
        let endpoint_limits = self
            .workflows
            .iter()
            .flat_map(|workflow| {
                workflow.endpoints().iter().map(move |endpoint| {
                    (
                        (workflow.id().to_string(), endpoint.id().to_string()),
                        endpoint.max_instances() as usize,
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        let mut current_ordinals = BTreeMap::<(String, String, u64), Vec<u16>>::new();
        for instance in &self.workflow_runtime_instances {
            if instance.status() == WorkflowEndpointRuntimeInstanceStatus::Stale {
                continue;
            }
            current_ordinals
                .entry((
                    instance.workflow_id().to_string(),
                    instance.endpoint_id().to_string(),
                    instance.workflow_revision(),
                ))
                .or_default()
                .push(instance.ordinal());
        }
        for ordinals in current_ordinals.values_mut() {
            ordinals.sort_unstable();
        }
        self.workflow_runtime_instances
            .iter()
            .filter_map(|instance| {
                let stale = instance.status() == WorkflowEndpointRuntimeInstanceStatus::Stale;
                let over_limit = endpoint_limits
                    .get(&(
                        instance.workflow_id().to_string(),
                        instance.endpoint_id().to_string(),
                    ))
                    .and_then(|limit| {
                        current_ordinals
                            .get(&(
                                instance.workflow_id().to_string(),
                                instance.endpoint_id().to_string(),
                                instance.workflow_revision(),
                            ))
                            .map(|ordinals| {
                                ordinals
                                    .iter()
                                    .position(|ordinal| *ordinal == instance.ordinal())
                                    .is_some_and(|index| index >= *limit)
                            })
                    })
                    .unwrap_or(true);
                let cleanup = (stale || over_limit)
                    && instance.status() != WorkflowEndpointRuntimeInstanceStatus::Busy;
                cleanup.then(|| instance.clone())
            })
            .collect()
    }

    fn workflow_run_ids_pending_prompt_settlement(&self) -> BTreeSet<String> {
        self.prompt_runtime
            .prompt_states()
            .values()
            .flat_map(|state| {
                state
                    .active_prompt()
                    .into_iter()
                    .chain(state.queued_prompts())
            })
            .filter_map(|prompt| prompt.workflow_run_id().map(str::to_string))
            .chain(self.settling_workflow_run_counts.keys().cloned())
            .collect()
    }

    pub(crate) fn durable_runtime_snapshot(&self) -> Self {
        let pending_settlement = self.workflow_run_ids_pending_prompt_settlement();
        let mut snapshot = self.clone();
        snapshot.workflow_runs.retain(|workflow_run| {
            !workflow_run.status().is_terminal() || pending_settlement.contains(workflow_run.id())
        });
        snapshot
            .workflow_publication_state
            .workflow_event_delivery_receipts
            .clear();
        snapshot
    }

    pub(crate) fn archive_terminal_workflow_runs(&mut self) -> Vec<WorkflowRun> {
        let pending_settlement = self.workflow_run_ids_pending_prompt_settlement();
        let mut active = Vec::with_capacity(self.workflow_runs.len());
        let mut archived = Vec::new();
        for workflow_run in self.workflow_runs.drain(..) {
            if workflow_run.status().is_terminal()
                && !pending_settlement.contains(workflow_run.id())
            {
                archived.push(workflow_run);
            } else {
                active.push(workflow_run);
            }
        }
        self.workflow_runs = active;
        archived
    }

    pub(crate) fn restore_active_workflow_runs(&mut self, workflow_runs: Vec<WorkflowRun>) {
        self.workflow_runs
            .retain(|workflow_run| workflow_run.status().is_terminal());
        for workflow_run in workflow_runs {
            if workflow_run.status().is_terminal() {
                continue;
            }
            match self
                .workflow_runs
                .iter()
                .position(|current| current.id() == workflow_run.id())
            {
                Some(index) => self.workflow_runs[index] = workflow_run,
                None => self.workflow_runs.push(workflow_run),
            }
        }
    }

    pub(crate) fn restore_workflow_event_delivery_receipts(
        &mut self,
        receipts: impl IntoIterator<Item = WorkflowEventDeliveryReceipt>,
    ) {
        self.workflow_publication_state
            .workflow_event_delivery_receipts
            .extend(
                receipts
                    .into_iter()
                    .map(|receipt| (receipt.delivery_id.clone(), receipt)),
            );
    }

    pub fn has_active_workflow_run(&self) -> bool {
        self.workflow_runs.iter().any(|workflow_run| {
            matches!(
                workflow_run.status(),
                WorkflowRunStatus::Created
                    | WorkflowRunStatus::Running
                    | WorkflowRunStatus::Waiting
                    | WorkflowRunStatus::Paused
            )
        })
    }

    /// Mark the workflow run before removing its active provider prompt. The
    /// provider settlement path performs asynchronous post-processing after
    /// that removal (for example git observation); live orphan reconciliation
    /// must not mistake that short gap for a lost workflow.
    pub fn mark_workflow_run_settling(&mut self, workflow_run_id: &str) -> bool {
        if !self
            .workflow_runs
            .iter()
            .any(|workflow_run| workflow_run.id() == workflow_run_id)
        {
            return false;
        }
        *self
            .settling_workflow_run_counts
            .entry(workflow_run_id.to_string())
            .or_default() += 1;
        true
    }

    pub fn clear_workflow_run_settling(&mut self, workflow_run_id: &str) {
        let Some(count) = self.settling_workflow_run_counts.get_mut(workflow_run_id) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.settling_workflow_run_counts.remove(workflow_run_id);
        }
    }

    pub fn is_workflow_run_settling(&self, workflow_run_id: &str) -> bool {
        self.settling_workflow_run_counts
            .contains_key(workflow_run_id)
    }

    pub fn reconcile_after_kernel_restart(&mut self) -> KernelRestartReconciliation {
        let mut reconciliation = KernelRestartReconciliation::default();
        reconciliation.removed_orphaned_workflow_prompt_count =
            self.reconcile_workflow_queue_ownership();
        if self.active_provider_run_id.take().is_some() {
            reconciliation.cleared_active_provider_run = true;
        }
        reconciliation.cleared_attachment_count = self.clear_attachments();
        let terminal_workflow_run_ids = self
            .workflow_runs
            .iter()
            .filter(|workflow_run| {
                matches!(
                    workflow_run.status(),
                    WorkflowRunStatus::Completed
                        | WorkflowRunStatus::Failed
                        | WorkflowRunStatus::Stopped
                )
            })
            .map(|workflow_run| workflow_run.id().to_string())
            .collect::<BTreeSet<_>>();
        reconciliation.removed_terminal_workflow_prompt_count =
            self.prompt_runtime.remove_active_prompts_by_workflow_runs(
                &terminal_workflow_run_ids,
                self.focused_agent_id.as_deref(),
            );
        reconciliation.recoverable_prompt_count = self
            .prompt_runtime
            .prompt_states()
            .values()
            .filter(|state| state.active_prompt().is_some())
            .count();
        reconciliation.recoverable_workflow_run_count = self
            .workflow_runs
            .iter()
            .filter(|workflow_run| {
                !matches!(
                    workflow_run.status(),
                    WorkflowRunStatus::Completed
                        | WorkflowRunStatus::Failed
                        | WorkflowRunStatus::Stopped
                        | WorkflowRunStatus::Paused
                )
            })
            .count();

        // A previous kernel could have admitted a workflow prompt and persisted the
        // provider-side prompt as Dispatching before persisting the workflow node's
        // Running/Dispatched transition.  That state is not orphaned: the active
        // provider prompt is the durable proof that this node owns the session lane.
        // Repair the workflow projection before queue arbitration runs, otherwise
        // `has_active_workflow_run` sees a Created run and every later event remains
        // queued indefinitely.
        let active_workflow_prompts = self
            .prompt_runtime
            .prompt_states()
            .values()
            .filter_map(|state| state.active_prompt().cloned())
            .filter(|prompt| {
                matches!(
                    prompt.status(),
                    crate::session::PromptStatus::Dispatching
                        | crate::session::PromptStatus::Running
                )
            })
            .filter_map(|prompt| {
                Some((
                    prompt.workflow_run_id()?.to_string(),
                    prompt.workflow_node_run_id()?.to_string(),
                ))
            })
            .collect::<Vec<_>>();
        for (workflow_run_id, workflow_node_run_id) in active_workflow_prompts {
            let Some((workflow_run_status, node_run_status, envelope_state)) = self
                .workflow_run(&workflow_run_id)
                .and_then(|workflow_run| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| node_run.id() == workflow_node_run_id)
                        .map(|node_run| {
                            (
                                workflow_run.status(),
                                node_run.status(),
                                node_run.turn_envelope().map(|envelope| envelope.state()),
                            )
                        })
                })
            else {
                continue;
            };
            // Only repair the pre-start projection that can be left behind by a
            // crash between prompt admission and workflow start.  A terminal
            // workflow/node/envelope is authoritative and must never be
            // resurrected merely because its prompt cleanup was interrupted.
            if matches!(
                workflow_run_status,
                WorkflowRunStatus::Completed
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Stopped
            ) || node_run_status != WorkflowNodeRunStatus::Ready
                || !matches!(
                    envelope_state,
                    Some(
                        crate::session::WorkflowTurnRuntimeState::Prepared
                            | crate::session::WorkflowTurnRuntimeState::Dispatched
                    )
                )
            {
                continue;
            }
            let Some(workflow_run) = self.workflow_run_mut(&workflow_run_id) else {
                continue;
            };
            let Some(node_run) = workflow_run.node_run_mut(&workflow_node_run_id) else {
                continue;
            };
            if let Some(envelope) = node_run.turn_envelope_mut() {
                if envelope.state() == crate::session::WorkflowTurnRuntimeState::Prepared {
                    envelope.mark_dispatched();
                }
            }
            node_run.set_status(WorkflowNodeRunStatus::Running);
            workflow_run.set_active_node_run(workflow_node_run_id);
            workflow_run.set_status(WorkflowRunStatus::Running);
            reconciliation.repaired_workflow_prompt_count += 1;
        }

        let durable_workflow_prompt_targets = self
            .prompt_runtime
            .prompt_states()
            .values()
            .flat_map(|state| {
                state
                    .active_prompt()
                    .into_iter()
                    .chain(state.queued_prompts())
            })
            .filter_map(|prompt| {
                Some((
                    prompt.workflow_run_id()?.to_string(),
                    prompt.workflow_node_run_id()?.to_string(),
                ))
            })
            .collect::<BTreeSet<_>>();
        let durable_workflow_run_targets = self
            .workflow_queued_prompts
            .iter()
            .filter(|queued_prompt| {
                matches!(
                    queued_prompt.status(),
                    WorkflowQueuedPromptStatus::Dispatching | WorkflowQueuedPromptStatus::Running
                )
            })
            .filter_map(|queued_prompt| queued_prompt.workflow_run_id().map(str::to_string))
            .collect::<BTreeSet<_>>();
        let mut orphaned_workflow_run_ids = Vec::new();
        for workflow_run in &mut self.workflow_runs {
            if !matches!(
                workflow_run.status(),
                WorkflowRunStatus::Created
                    | WorkflowRunStatus::Running
                    | WorkflowRunStatus::Waiting
            ) {
                continue;
            }
            if durable_workflow_prompt_targets
                .iter()
                .any(|(run_id, _)| run_id == workflow_run.id())
                || durable_workflow_run_targets.contains(workflow_run.id())
            {
                continue;
            }
            // A non-terminal workflow run must have a durable provider prompt (or a durable
            // dispatching queue record). If neither exists, the provider turn was lost between
            // snapshots (for example after an acknowledged provider turn exited before recording
            // completion). Leaving the run Running/Waiting would retain a workflow that can never
            // make progress.
            // Workspace claims are process-local and are retried by the runtime recovery pass;
            // do not classify a blocked node as orphaned before that pass gets a chance to
            // reacquire its claim and dispatch the prepared prompt.
            if workflow_run
                .node_runs()
                .iter()
                .any(|node| node.status() == WorkflowNodeRunStatus::BlockedOnWorkspaceClaim)
            {
                continue;
            }
            let orphaned_node_run_ids = workflow_run
                .node_runs()
                .iter()
                .filter(|node| {
                    !matches!(
                        node.status(),
                        WorkflowNodeRunStatus::Completed
                            | WorkflowNodeRunStatus::Failed
                            | WorkflowNodeRunStatus::Stopped
                    )
                })
                .map(|node| node.id().to_string())
                .collect::<Vec<_>>();
            for node_run in workflow_run.node_runs_mut() {
                if !orphaned_node_run_ids.iter().any(|id| id == node_run.id()) {
                    continue;
                }
                node_run.set_status(WorkflowNodeRunStatus::Stopped);
                if let Some(envelope) = node_run.turn_envelope_mut() {
                    envelope.mark_cancelled();
                }
            }
            let failure_source_node_run_id = workflow_run
                .active_node_run_id()
                .map(str::to_string)
                .or_else(|| orphaned_node_run_ids.first().cloned())
                .unwrap_or_else(|| workflow_run.id().to_string());
            workflow_run.clear_active_node_run();
            workflow_run.add_failure_event(WorkflowFailureEvent::new(
                WorkflowFailureKind::RunStopped,
                failure_source_node_run_id,
                Vec::new(),
                "non-terminal workflow run had no durable active or queued prompt after kernel restart",
            ));
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            orphaned_workflow_run_ids.push(workflow_run.id().to_string());
            reconciliation.stopped_workflow_run_count += 1;
        }
        for workflow_run_id in orphaned_workflow_run_ids {
            self.prompt_runtime.remove_queued_prompts_by_workflow_run(
                &workflow_run_id,
                self.focused_agent_id.as_deref(),
            );
        }

        self.reconcile_workflow_runtime_instances();

        reconciliation
    }

    pub(crate) fn interrupt_runtime_for_shutdown(&mut self) -> KernelRestartReconciliation {
        let mut reconciliation = self.reconcile_after_kernel_restart();

        reconciliation.interrupted_prompt_count = self
            .prompt_runtime
            .interrupt_active_prompts(self.focused_agent_id.as_deref())
            .len();

        let mut stopped_workflow_run_ids = Vec::new();
        for workflow_run in &mut self.workflow_runs {
            let should_stop = !matches!(
                workflow_run.status(),
                WorkflowRunStatus::Completed
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Stopped
                    | WorkflowRunStatus::Paused
            );
            if !should_stop {
                continue;
            }

            let source_node_run_id = workflow_run
                .active_node_run_id()
                .map(str::to_string)
                .or_else(|| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| {
                            !matches!(
                                node_run.status(),
                                WorkflowNodeRunStatus::Completed
                                    | WorkflowNodeRunStatus::Failed
                                    | WorkflowNodeRunStatus::Stopped
                            )
                        })
                        .map(|node_run| node_run.id().to_string())
                })
                .unwrap_or_else(|| workflow_run.id().to_string());

            for node_run in workflow_run.node_runs_mut() {
                if !matches!(
                    node_run.status(),
                    WorkflowNodeRunStatus::Completed
                        | WorkflowNodeRunStatus::Failed
                        | WorkflowNodeRunStatus::Stopped
                ) {
                    node_run.set_status(WorkflowNodeRunStatus::Stopped);
                    if let Some(envelope) = node_run.turn_envelope_mut() {
                        envelope.mark_cancelled();
                    }
                }
            }
            workflow_run.clear_active_node_run();
            workflow_run.add_failure_event(WorkflowFailureEvent::new(
                WorkflowFailureKind::RunStopped,
                source_node_run_id,
                Vec::new(),
                "workflow run was interrupted by kernel restart; relaunch or resume it explicitly",
            ));
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            stopped_workflow_run_ids.push(workflow_run.id().to_string());
            reconciliation.stopped_workflow_run_count += 1;
        }
        for workflow_run_id in stopped_workflow_run_ids {
            self.remove_queued_prompts_by_workflow_run(&workflow_run_id);
        }

        reconciliation
    }

    pub fn add_workflow_prompt_queue(
        &mut self,
        queue: WorkflowPromptQueueDefinition,
    ) -> WorkflowPromptQueueDefinition {
        self.workflow_prompt_queues.push(queue.clone());
        queue
    }

    pub fn workflow_prompt_queues_for_workflow(
        &self,
        workflow_id: &str,
    ) -> Vec<WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues
            .iter()
            .filter(|queue| queue.workflow_id() == workflow_id)
            .cloned()
            .collect()
    }

    pub fn workflow_prompt_queue(
        &self,
        workflow_id: &str,
        queue_id: &str,
    ) -> Option<&WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues.iter().find(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == queue_id || queue.alias() == queue_id)
        })
    }

    pub fn workflow_prompt_queue_mut(
        &mut self,
        workflow_id: &str,
        queue_id: &str,
    ) -> Option<&mut WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues.iter_mut().find(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == queue_id || queue.alias() == queue_id)
        })
    }

    pub fn remove_workflow_prompt_queue(
        &mut self,
        workflow_id: &str,
        queue_id: &str,
    ) -> Option<WorkflowPromptQueueDefinition> {
        let index = self.workflow_prompt_queues.iter().position(|queue| {
            queue.workflow_id() == workflow_id
                && (queue.id() == queue_id || queue.alias() == queue_id)
        })?;
        Some(self.workflow_prompt_queues.remove(index))
    }

    pub fn ensure_default_workflow_prompt_queue(&mut self, workflow_id: &str) {
        if self
            .workflow_prompt_queues
            .iter()
            .any(|queue| queue.workflow_id() == workflow_id && queue.alias() == "default")
        {
            return;
        }
        self.workflow_prompt_queues
            .push(WorkflowPromptQueueDefinition::default_queue(workflow_id));
    }

    pub fn enqueue_workflow_prompt(
        &mut self,
        queued_prompt: WorkflowQueuedPrompt,
    ) -> WorkflowQueuedPrompt {
        self.workflow_queued_prompts
            .push_back(queued_prompt.clone());
        queued_prompt
    }

    pub fn update_queued_workflow_prompt(
        &mut self,
        queue_item_id: &str,
        prompt: Option<String>,
        queue_id: Option<String>,
    ) -> Option<WorkflowQueuedPrompt> {
        let queued_prompt = self
            .workflow_queued_prompts
            .iter_mut()
            .find(|item| item.id() == queue_item_id)?;
        if queued_prompt.status() != WorkflowQueuedPromptStatus::Queued {
            return None;
        }
        if let Some(queue_id) = queue_id {
            queued_prompt.set_queue_id(queue_id);
        }
        queued_prompt.set_prompt(prompt);
        Some(queued_prompt.clone())
    }

    pub fn remove_queued_workflow_prompt(
        &mut self,
        queue_item_id: &str,
    ) -> Option<WorkflowQueuedPrompt> {
        let index = self
            .workflow_queued_prompts
            .iter()
            .position(|queued_prompt| queued_prompt.id() == queue_item_id)?;
        if self.workflow_queued_prompts[index].status() != WorkflowQueuedPromptStatus::Queued {
            return None;
        }
        self.workflow_queued_prompts.remove(index)
    }

    pub fn remove_queued_workflow_prompts_for_watchdog(
        &mut self,
        watchdog_id: &str,
    ) -> Vec<WorkflowQueuedPrompt> {
        let mut removed = Vec::new();
        self.workflow_queued_prompts.retain(|prompt| {
            if prompt.watchdog_id() == Some(watchdog_id) {
                removed.push(prompt.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    pub fn clear_workflow_queue(&mut self, queue_id: &str) -> Vec<WorkflowQueuedPrompt> {
        let mut removed = Vec::new();
        let mut kept = VecDeque::new();
        while let Some(item) = self.workflow_queued_prompts.pop_front() {
            if item.queue_id() == queue_id && item.status() == WorkflowQueuedPromptStatus::Queued {
                removed.push(item);
            } else {
                kept.push_back(item);
            }
        }
        self.workflow_queued_prompts = kept;
        removed
    }

    pub fn pop_next_workflow_queued_prompt(&mut self) -> Option<WorkflowQueuedPrompt> {
        let best = self
            .workflow_queued_prompts
            .iter()
            .enumerate()
            .filter(|(_, item)| item.status() == WorkflowQueuedPromptStatus::Queued)
            .filter_map(|(index, item)| {
                let queue = self.workflow_prompt_queue(item.workflow_id(), item.queue_id())?;
                if !queue.enabled() {
                    return None;
                }
                Some((index, queue.priority(), item.created_at_ms()))
            })
            .min_by_key(|(_, priority, created_at_ms)| {
                (std::cmp::Reverse(*priority), *created_at_ms)
            })
            .map(|(index, _, _)| index)?;
        let mut item = self.workflow_queued_prompts.remove(best)?;
        item.mark_dispatching();
        Some(item)
    }

    pub fn pop_next_workflow_queued_prompt_with_idle_instance(
        &mut self,
        workflow_run_id: &str,
    ) -> Option<(WorkflowQueuedPrompt, WorkflowEndpointRuntimeInstance)> {
        self.reconcile_workflow_runtime_instances();
        let best = self
            .workflow_queued_prompts
            .iter()
            .enumerate()
            .filter(|(_, item)| item.status() == WorkflowQueuedPromptStatus::Queued)
            .filter_map(|(index, item)| {
                let queue = self.workflow_prompt_queue(item.workflow_id(), item.queue_id())?;
                if !queue.enabled() {
                    return None;
                }
                let workflow = self.workflow(item.workflow_id())?;
                let instance = self.idle_workflow_runtime_instance(
                    workflow.id(),
                    item.endpoint_id(),
                    workflow.revision(),
                )?;
                Some((
                    index,
                    queue.priority(),
                    item.created_at_ms(),
                    instance.clone(),
                ))
            })
            .min_by_key(|(_, priority, created_at_ms, _)| {
                (std::cmp::Reverse(*priority), *created_at_ms)
            });
        let (index, _, _, instance) = best?;
        let instance = self
            .workflow_runtime_instances
            .iter_mut()
            .find(|candidate| candidate.id() == instance.id())
            .and_then(|candidate| candidate.claim(workflow_run_id).then(|| candidate.clone()))?;
        let mut item = self.workflow_queued_prompts.remove(index)?;
        item.mark_dispatching();
        Some((item, instance))
    }

    pub fn next_dispatchable_workflow_queued_prompt_created_at_ms(&self) -> Option<u64> {
        self.workflow_queued_prompts
            .iter()
            .filter(|item| item.status() == WorkflowQueuedPromptStatus::Queued)
            .filter_map(|item| {
                let queue = self.workflow_prompt_queue(item.workflow_id(), item.queue_id())?;
                let workflow = self.workflow(item.workflow_id())?;
                self.idle_workflow_runtime_instance(
                    workflow.id(),
                    item.endpoint_id(),
                    workflow.revision(),
                )?;
                queue
                    .enabled()
                    .then_some((queue.priority(), item.created_at_ms()))
            })
            .min_by_key(|(priority, created_at_ms)| (std::cmp::Reverse(*priority), *created_at_ms))
            .map(|(_, created_at_ms)| created_at_ms)
    }

    pub fn next_workflow_queued_prompt_created_at_ms(&self) -> Option<u64> {
        self.workflow_queued_prompts
            .iter()
            .filter(|item| item.status() == WorkflowQueuedPromptStatus::Queued)
            .filter_map(|item| {
                let queue = self.workflow_prompt_queue(item.workflow_id(), item.queue_id())?;
                queue
                    .enabled()
                    .then_some((queue.priority(), item.created_at_ms()))
            })
            .min_by_key(|(priority, created_at_ms)| (std::cmp::Reverse(*priority), *created_at_ms))
            .map(|(_, created_at_ms)| created_at_ms)
    }

    pub fn workflow_run(&self, workflow_run_id: &str) -> Option<&WorkflowRun> {
        self.workflow_runs
            .iter()
            .find(|workflow_run| workflow_run.id() == workflow_run_id)
    }

    pub fn workflow_run_mut(&mut self, workflow_run_id: &str) -> Option<&mut WorkflowRun> {
        self.workflow_runs
            .iter_mut()
            .find(|workflow_run| workflow_run.id() == workflow_run_id)
    }

    pub fn add_workflow_schedule(
        &mut self,
        schedule: WorkflowScheduleDefinition,
    ) -> WorkflowScheduleDefinition {
        self.workflow_schedules.push(schedule.clone());
        schedule
    }

    pub fn workflow_schedule(&self, schedule_id: &str) -> Option<&WorkflowScheduleDefinition> {
        self.workflow_schedules
            .iter()
            .find(|schedule| schedule.id() == schedule_id)
    }

    pub fn workflow_schedule_mut(
        &mut self,
        schedule_id: &str,
    ) -> Option<&mut WorkflowScheduleDefinition> {
        self.workflow_schedules
            .iter_mut()
            .find(|schedule| schedule.id() == schedule_id)
    }

    pub fn remove_workflow_schedule(
        &mut self,
        schedule_id: &str,
    ) -> Option<WorkflowScheduleDefinition> {
        let index = self
            .workflow_schedules
            .iter()
            .position(|schedule| schedule.id() == schedule_id)?;
        Some(self.workflow_schedules.remove(index))
    }

    pub fn add_workflow_watchdog(
        &mut self,
        watchdog: WorkflowWatchdogDefinition,
    ) -> WorkflowWatchdogDefinition {
        self.add_workflow_schedule(watchdog)
    }

    pub fn workflow_watchdog(&self, watchdog_id: &str) -> Option<&WorkflowWatchdogDefinition> {
        self.workflow_schedule(watchdog_id)
    }

    pub fn workflow_watchdog_mut(
        &mut self,
        watchdog_id: &str,
    ) -> Option<&mut WorkflowWatchdogDefinition> {
        self.workflow_schedule_mut(watchdog_id)
    }

    pub fn remove_workflow_watchdog(
        &mut self,
        watchdog_id: &str,
    ) -> Option<WorkflowWatchdogDefinition> {
        self.remove_workflow_schedule(watchdog_id)
    }

    pub fn workflow_node_run_mut(
        &mut self,
        workflow_node_run_id: &str,
    ) -> Option<&mut WorkflowNodeRun> {
        self.workflow_runs
            .iter_mut()
            .find_map(|workflow_run| workflow_run.node_run_mut(workflow_node_run_id))
    }

    pub fn workflow_console(&self, workflow_id: &str) -> Option<&WorkflowConsole> {
        self.workflow_consoles
            .iter()
            .find(|console| console.workflow_id() == workflow_id)
    }

    pub fn workflow_console_mut(&mut self, workflow_id: &str) -> Option<&mut WorkflowConsole> {
        self.workflow_consoles
            .iter_mut()
            .find(|console| console.workflow_id() == workflow_id)
    }

    pub fn ensure_workflow_console(
        &mut self,
        workflow_id: impl Into<String>,
    ) -> &mut WorkflowConsole {
        let workflow_id = workflow_id.into();
        if let Some(index) = self
            .workflow_consoles
            .iter()
            .position(|console| console.workflow_id() == workflow_id)
        {
            return &mut self.workflow_consoles[index];
        }
        self.workflow_consoles
            .push(WorkflowConsole::new(workflow_id));
        let index = self.workflow_consoles.len() - 1;
        &mut self.workflow_consoles[index]
    }
}
