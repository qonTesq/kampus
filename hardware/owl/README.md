# Owl: hardware

The physical build of the talking owl mascot.

## Layout

```
hardware/owl/
├── cad/          source models (STEP, Fusion/FreeCAD files)
├── print/        printable STL/3MF files and print settings
├── wiring/       wiring diagrams and pin maps
├── photos/       build photos used in the docs
└── bom.md        bill of materials (parts, quantities, links)
```

## Parts (summary)

| Part | Role |
|---|---|
| Raspberry Pi 5 (8 GB) | Runs the owl client |
| 2× GC9A01 1.28" round displays | Eyes (SPI) |
| USB webcam | Microphone (camera off) |
| Bluetooth speaker | Voice |

Put the full list with links in `bom.md`.

## Large files

CAD and print files are stored with **Git LFS** (see the root `.gitattributes`). Before adding them, install it once:

```bash
git lfs install
```

The software side lives in [`clients/owl/`](../../clients/owl/).
