# Introduction

Very simple library for webcam native video capture that uses [Nokhwa](https://github.com/l1npengtul/nokhwa) to export basic C functions so they can be called from any language such as Java, Python, etc.

This was built to be used with JNA in https://github.com/eduramiba/webcam-capture-driver-native

## Active capture format

After starting capture, call `cnokhwa_active_format(uint32_t device, uint32_t *width, uint32_t *height, double *fps, char *pixel_format, size_t capacity)` on Linux, Windows or macOS. It returns 0 on success and writes a NUL-terminated format name. A frame rate of 0 means the backend could not establish the negotiated interval. The existing frame size and RGB stride functions describe the same session.

The Cargo override in `vendor/nokhwa-bindings-linux` patches the V4L2 constructor that rejects driver-adjusted modes in upstream 0.1.4. It also exposes read-only format enumeration and the negotiated frame interval. The Windows and macOS backends use their published Nokhwa bindings.
