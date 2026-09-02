# Kaifuku — NSC Version

**Kaifuku** is a data recovery and binary analysis application developed as a project for the **NSC (National Software Contest)**.

This repository contains the **NSC competition version** of Kaifuku. It integrates the PhotoRec recovery engine with a Rust-based graphical application through a Rust–C FFI layer and provides additional tools for managing and analyzing recovered data.

> **Note:** This repository represents the NSC version of Kaifuku and is not the final architecture of the project.

## Features

### Data Recovery

Kaifuku provides a graphical workflow for recovering files from storage devices using PhotoRec.

The recovery workflow includes:

- Storage device selection
- Partition selection
- Recovery destination selection
- Recovery configuration
- Recovery progress monitoring
- Recovered-file organization

### PhotoRec Integration

The NSC version integrates PhotoRec directly into the Kaifuku application.

Instead of treating PhotoRec only as an external command-line program, Kaifuku connects its Rust application layer with the C-based PhotoRec components through **Foreign Function Interface (FFI)**.

```text
┌──────────────────────────────┐
│          Kaifuku GUI         │
│         Rust + FLTK          │
└──────────────┬───────────────┘
               │
               │ Rust ↔ C FFI
               ▼
┌──────────────────────────────┐
│      PhotoRec Components     │
│              C               │
└──────────────┬───────────────┘
               │
               ▼
┌──────────────────────────────┐
│       File Recovery          │
│          PhotoRec            │
└──────────────────────────────┘
```

The `c_src/` directory contains the C components used by the NSC version, including PhotoRec/TestDisk source code and Kaifuku-specific integration code.

### Disk Imaging

Kaifuku supports creating disk images before performing recovery.

A disk image can be used as the recovery source, allowing recovery operations to be performed without repeatedly accessing the original storage device.

```text
Storage Device
      │
      │ Disk Imaging
      ▼
   DD Image
      │
      │ Recovery
      ▼
Recovered Files
```

This workflow can be useful when working with important data because it allows the recovery process to operate on an image of the source.

### Recovery Result Management

Recovered files can be organized into categories to make large recovery results easier to navigate.

Supported categories include:

- Pictures
- Videos
- Documents
- Other

### Binary Analysis

Kaifuku includes tools for inspecting recovered files at the binary level.

These tools are intended to help users examine file contents, signatures, offsets, and binary structures.

#### Hex Editor

The Hex Editor provides a byte-level view of a file.

It includes:

- Hexadecimal representation
- ASCII representation
- Byte offsets
- File navigation
- Save
- Save As

#### Signature Scanner

The Signature Scanner searches binary data for known file signatures.

It can display:

- Detected signatures
- File types
- Signature offsets

This can help users investigate the contents of recovered files and identify recognizable binary structures.

#### Header Template Generator

The Header Template Generator provides templates related to supported file headers and structures.

These templates can be used as references when examining binary data.

### Recovery Log

Kaifuku provides logging for recovery-related operations.

The log can help users track activities performed during the recovery workflow.

## Architecture

The NSC version consists primarily of a Rust application layer combined with C-based PhotoRec components.

```text
                    Kaifuku
                       │
        ┌──────────────┴──────────────┐
        │                             │
        ▼                             ▼
   Rust Application             Binary Analysis
        │
        │
        ▼
   Rust–C FFI Layer
        │
        ▼
  Modified / Integrated
      PhotoRec
        │
        ▼
   File Recovery Engine
```

### Main Technologies

| Component | Technology |
|---|---|
| Application | Rust |
| GUI | FLTK / FLTK-RS |
| Recovery Engine | PhotoRec |
| Integration | Rust–C FFI |
| Disk Imaging | `dd` |
| Partition Management | GParted |
| Build System | Cargo |

## Project Structure

A simplified view of the repository:

```text
Kaifuku/
├── c_src/
│   ├── PhotoRec / TestDisk C sources
│   ├── Kaifuku FFI integration
│   └── modified C components
│
├── src/
│   ├── backend/
│   │   ├── ffi.rs
│   │   ├── photorec.rs
│   │   └── ...
│   ├── pages/
│   ├── utils/
│   └── ...
│
├── Cargo.toml
├── build.rs
├── LICENSE
└── README.md
```

## Recovery Workflow

A typical recovery process is:

```text
1. Select storage device
          ↓
2. Create disk image (optional)
          ↓
3. Select recovery source
          ↓
4. Configure recovery
          ↓
5. Start recovery
          ↓
6. Organize recovered files
          ↓
7. Analyze recovered data
```

For important recovery operations, creating an image before recovery is recommended.

## Requirements

The NSC version is designed for a 64-bit Linux environment.

Recommended:

- 64-bit Intel or AMD processor
- At least 2 GB RAM
- Sufficient storage for recovered data
- Sufficient storage for disk images when using disk imaging

Additional dependencies may be required depending on the Linux distribution and current build configuration.

## Building

Install Rust and Cargo, then clone the repository:

```bash
git clone https://github.com/<your-username>/kaifuku.git
cd kaifuku
```

Build the project:

```bash
cargo build --release
```

Run Kaifuku:

```bash
cargo run --release
```

The exact system dependencies may vary depending on the Linux distribution and project configuration.

## Recovery Limitations

Kaifuku's NSC version uses PhotoRec for file recovery, so recovery results depend on the underlying storage medium and the capabilities of the recovery engine.

Recovery cannot be guaranteed when:

- Data has been overwritten
- Required data is no longer readable
- The storage device has severe hardware problems
- File data cannot be identified by the recovery engine
- The original storage medium has been significantly damaged

### Important

Do **not** save recovered files to the same storage device from which you are attempting to recover data whenever possible.

Writing data to the source device may overwrite data that could otherwise be recovered.

## NSC Version

This repository represents the version of Kaifuku developed and submitted for the **NSC competition**.

The project was developed with a focus on:

- Data recovery
- PhotoRec integration
- Rust–C FFI
- Disk imaging
- Recovery result management
- Binary analysis
- Recovery-oriented tooling

The architecture and implementation may differ from later versions of the project.

## Third-Party Components

This project incorporates components from the **PhotoRec/TestDisk** project developed by **CGSecurity**.

The `c_src/` directory contains third-party source code as well as Kaifuku-specific integration and modifications.

Original copyright notices and applicable license information for third-party components are retained.

Please refer to the applicable license information included with the relevant third-party source code.

## License

Kaifuku is licensed under the **GNU General Public License v3.0 (GPL-3.0)**.

See the [`LICENSE`](LICENSE) file for the full license text.

Third-party components, including PhotoRec/TestDisk source code, remain subject to their applicable copyright and license terms.

## Disclaimer

Kaifuku is intended for legitimate data recovery, research, educational, and system-administration purposes.

Only use Kaifuku on storage devices or data that you own or have permission to analyze.

Data recovery is not guaranteed. Recovery operations on unstable or physically damaged storage devices may result in further data loss.

## Acknowledgements

- **CGSecurity / PhotoRec** — file recovery technology
- **Rust** — application development
- **FLTK** — graphical user interface
- **Linux ecosystem** — system and storage utilities

---

**Kaifuku — NSC Competition Version**
