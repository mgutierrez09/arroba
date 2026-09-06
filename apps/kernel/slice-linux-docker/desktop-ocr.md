# Desktop text recognition

Text reading and text lookup share `slice-text-finder.py --image`. The helper
combines Tesseract's automatic page segmentation with a block pass. Automatic
segmentation alone can find a taskbar while completely missing readable text
inside a small window on a dark desktop.

Both passes use the original image. Lookup coordinates remain desktop pixels;
there is no crop, scale, window movement, or focus change. Overlapping detections
of the same target are coalesced, while separate occurrences remain separate.
Text reading removes duplicate lines. Recognition remains approximate, not a
guarantee of transcription accuracy or complete reading-order reconstruction.

TSV stays in memory per invocation, so concurrent lookups do not share a scratch
file. Each subprocess has a 15-second timeout. The older QUERY TSV entry point
remains available for saved-launcher compatibility during support refresh.

The focused tests exercise the public lookup command with controlled OCR output.
The live desktop-settings drill checks real editor rendering, text lookup,
original-pixel coordinates and non-duplicated targets on startup, restart and
saved-home restoration. Evidence images remain outside Git.
