# site-server test data

- `tiny-2x1.avif`: 311-byte, 2x1 px solid fill (brand purple), encoded with
  Pillow's AVIF plugin. Synthetic, no member or game images. Used by the
  AVIF upload tests (sniff, header dimension read, inline serving). Tests that
  need other dimensions patch its single `ispe` box in memory.
