//! Shared frame pixels: board geometry that both readers (the cv reader and
//! Tesseract) and the privacy blanker use. Nothing here reads text, calls
//! Tesseract or touches packs.
//!
//! Any `#[cfg(test)] mod …;` goes at the bottom of this file: the OCR gate
//! source scan cuts each file at its first `#[cfg(test)]` module.

pub mod geometry;
