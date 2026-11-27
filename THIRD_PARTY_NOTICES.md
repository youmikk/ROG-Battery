# Third-party notices

## VigilWatt

Repository: https://github.com/evren-oct/vigil-watt
Revision: c97adc389b60b0e945a62cddacbaaace39887723

Protocol parsing/transport research and recorded regression fixtures for THUNDEROBOT ML903 NearLink Se and AKG N9 Hybrid refer to VigilWatt's Thunderobot and Akg modules, tests and hardware documentation. Upstream credits KNA7022 (PR #25) and endermiteDOTcn (PR #44). Our hardware transport, UI and integration are maintained separately. See docs/PROTOCOLS.md for evidence and validation boundaries.

LANGTU T98 protocol research and reconstructed battery prefixes refer to VigilWatt's Langtu module and hardware documentation, credited upstream to YyingnLL / haloneko. The local Rust adapter and lifecycle integration are independently implemented; reconstructed vectors are distinguished from complete recorded frames.

GANSS GS3104T-Pro protocol research and the recorded 97% regression fixture refer to VigilWatt's Ganss module and hardware documentation, credited upstream to luze258 (PR #15). The local adapter extends the existing independently implemented AULA status transport.

MIT License

Copyright (c) 2026 evren

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
