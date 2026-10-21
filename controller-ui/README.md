# ASCII Cam controller UI

Rust / eframe 0.31.1. Power and filter controls remain UI-only.

## Run on Windows

```powershell
cd C:\Users\jkl27\ascii-cam\controller-ui
cargo run
```

## Connected devices

- Queries Windows Media Foundation video capture devices in a background thread at launch.
- Refresh reloads the list after a camera is plugged in or removed. No automatic device-change monitoring.
- Lists hardware sources only (`MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_HW_SOURCE` nonzero). Software and unclassified sources are omitted.
- Also excludes known virtual-camera names, including ASCII Cam, OBS Virtual Camera and ManyCam, and the ASCII Cam CLSID when present in a device path.
- Driver metadata is not a universal physical-device guarantee. Virtual drivers that report hardware status and use an unrecognized name may appear; physical devices without hardware classification may be omitted. Extend the policy in `src/devices.rs` when testing additional drivers.
- Identifies devices by symbolic link so duplicate display names do not share selection state. Retains selection across a refresh only if that same device is still listed; otherwise clears it.
- Shows loading, empty, failure and unsupported-platform states. Selection never opens a camera, changes the C++ host's input camera or loads the camera DLL.
- Device discovery does not prove that a camera is free or can stream. A device may be in use by another app.

## Verify on Windows

1. Run `cargo run`. Confirm the built-in/USB camera name appears without starting video capture.
2. With ASCII Cam/OBS Virtual Camera installed, confirm their devices do not appear.
3. Connect a USB webcam, click Refresh and confirm its name appears.
4. Select it, disconnect it, then Refresh. Confirm it disappears and selection clears.
5. Refresh with the selected device still connected. Selection should remain.
6. Power and filter clicks should still change only the UI.

Cross-platform filter-policy tests: `rustc --edition=2021 --test src/devices.rs -o devices-tests` and run the resulting test executable. Windows API compilation can be checked with `cargo check --target x86_64-pc-windows-msvc`.

References:
- https://learn.microsoft.com/en-us/windows/win32/api/mfidl/nf-mfidl-mfenumdevicesources
- https://learn.microsoft.com/en-us/windows/win32/medfound/mf-devsource-attribute-source-type-vidcap-hw-source
