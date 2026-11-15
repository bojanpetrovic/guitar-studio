# Third-party notices

## ASIO (Windows build)

ASIO is a trademark and software of Steinberg Media Technologies GmbH.

The Windows build of Guitar Studio talks to ASIO drivers through Steinberg's ASIO SDK
2.3.4, which Steinberg licenses either under its proprietary ASIO licence or under the
GNU General Public License, version 3. Guitar Studio uses it under the **GPLv3**, so the
Windows installer, which contains the SDK's code, is distributed under the GPLv3 terms
(see `GPL-3.0.txt`). Its complete source is at https://github.com/bojanpetrovic/guitar-studio.

Guitar Studio's own code is under the MIT licence (`LICENSE`), which is compatible with
the GPLv3. The macOS build and the source code on their own don't include the ASIO SDK.
