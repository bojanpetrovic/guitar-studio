## Download

| | |
|---|---|
| **macOS** 13 Ventura or newer, Apple silicon or Intel | `Guitar-Studio-…-macOS.pkg` |
| **Windows** 10 or 11, 64-bit | `Guitar-Studio-…-Windows-setup.exe` |

Each installer has the desktop app and the VST3 and CLAP plugins. New to it? See the
[user manual](https://github.com/bojanpetrovic/guitar-studio/blob/master/docs/manual/README.md).

### macOS

These builds aren't signed with an Apple Developer ID, so macOS asks once:

1. Open the `.pkg`. macOS says it can't verify it: click **Done**.
2. Open **System Settings → Privacy & Security**, scroll down to the message about
   the installer and click **Open Anyway**, then confirm.
3. Install. **Customize** lets you skip the app or a plugin format. The app goes to
   Applications and the plugins to `/Library/Audio/Plug-Ins/VST3` and `…/CLAP`.

### Windows

The installer isn't signed either, so SmartScreen may say *Windows protected your PC*.
Click **More info → Run anyway**. It installs the app, the VST3 plugin in
`C:\Program Files\Common Files\VST3` and the CLAP plugin in `…\Common Files\CLAP\Guitar Studio`.

**ASIO:** for the lowest latency, pick your interface's ASIO driver in Audio settings.
ASIO drivers are listed as "ASIO: …", next to the normal Windows devices. An ASIO driver
handles input and output together, and its buffer size and sample rate may only change in
the driver's own control panel. Computers without an interface's ASIO driver can use
FlexASIO or ASIO4ALL.

This is the first Windows build, so please report anything that doesn't work.

### License

Guitar Studio is MIT-licensed. The Windows installer also contains Steinberg's ASIO SDK,
used under the GNU GPL v3, so the Windows build as a whole is distributed under the
GPLv3. The licence texts are installed with it (`licenses\`). ASIO is a trademark and
software of Steinberg Media Technologies GmbH.

### In a DAW

Rescan plugins (REAPER: Preferences → Plug-ins → VST → Re-scan; CLAP is picked up on
its own), then put **Guitar Studio** on a track with your guitar as its input.
