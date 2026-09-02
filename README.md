# Kaifuku

**Kaifuku** is a data recovery and binary analysis application built around [PhotoRec](https://www.cgsecurity.org/wiki/PhotoRec). It provides a graphical interface for data recovery while integrating PhotoRec directly with a Rust-based application through a **Rust–C Foreign Function Interface (FFI)**.

The project is designed to make file recovery easier to manage while providing additional tools for users who need to inspect recovered data at the binary level.

## Features

- **Graphical Data Recovery**
  - Recover files using PhotoRec through a graphical interface
  - Select storage devices and partitions
  - Configure recovery destinations
  - Monitor recovery progress

- **PhotoRec Integration**
  - Integrates PhotoRec with Kaifuku through Rust–C FFI
  - Uses PhotoRec's file-carving recovery engine
  - Modified PhotoRec source for direct integration with Kaifuku

- **Disk Imaging**
  - Create DD disk images before recovery
  - Use an existing disk image as the recovery source
  - Perform recovery operations from an image without repeatedly accessing the original device

- **Recovery Result Management**
  - Organize recovered files by category
  - Supported categories include:
    - Pictures
    - Videos
    - Documents
    - Other

- **Binary Analysis**
  - Inspect files at the byte level
  - Analyze file signatures and magic numbers
  - Inspect file offsets
  - Analyze binary structures and metadata

- **Advanced Mode**
  - Hex Editor
  - Signature Scanner
  - Header Template Generator
  - Recovery Log

## Architecture

Kaifuku is primarily written in **Rust**, while PhotoRec is written in **C**.

The application uses a Rust–C Foreign Function Interface to connect the Kaifuku application layer with the PhotoRec recovery engine.

```text
┌──────────────────────────────┐
│          Kaifuku GUI         │
│         Rust + FLTK          │
└──────────────┬───────────────┘
               │
               ▼
┌──────────────────────────────┐
│        Recovery Manager      │
│             Rust             │
└──────────────┬───────────────┘
               │
               │ Rust ↔ C FFI
               ▼
┌──────────────────────────────┐
│      Modified PhotoRec       │
│              C               │
└──────────────┬───────────────┘
               │
               ▼
┌──────────────────────────────┐
│       File Carving Engine    │
│           PhotoRec           │
└──────────────────────────────┘
```

Additional components provide disk imaging, binary analysis, recovery-result organization, and advanced analysis tools.

## Recovery Workflow

A typical recovery workflow is:

```text
Select Storage Device
        │
        ▼
Create Disk Image (Optional)
        │
        ▼
Select Recovery Source
        │
        ▼
Configure Recovery
        │
        ▼
Run PhotoRec
        │
        ▼
Recover Files
        │
        ▼
Organize Results
        │
        ▼
Analyze Recovered Files
```

When a disk image is available, it can be used as the recovery source instead of accessing the original storage device directly.

## Binary Analysis Tools

### Hex Editor

The Hex Editor allows users to inspect binary data in hexadecimal and ASCII representations.

Features include:

- Hexadecimal view
- ASCII view
- Offset information
- Byte-level editing
- Save
- Save As

It is intended for users who need to inspect or manually modify binary data.

### Signature Scanner

The Signature Scanner searches binary data for known file signatures or magic numbers.

It can:

- Detect file signatures
- Display the offset where a signature was found
- Identify detected file types
- Help locate embedded or nested file data

This can be useful when investigating files with damaged or unusual structures.

### Header Template Generator

The Header Template Generator creates templates for file-structure components based on supported formats.

Templates can be used as references during binary analysis or together with the Hex Editor.

### Recovery Log

Kaifuku provides a recovery log for recording important operations during the recovery process.

Logged information can include:

- Operation timestamps
- Disk image creation
- Recovery start
- Recovery status
- File output operations
- Analysis results

## Supported Storage

Kaifuku can work with storage devices detected by the underlying system, including:

- HDD
- SSD
- USB Flash Drive
- Memory Card

Recovery can also be performed from supported disk images such as DD images.

## Supported File Systems

Because Kaifuku uses PhotoRec as its primary recovery engine, supported file systems depend largely on PhotoRec's capabilities.

Examples include:

- FAT16
- FAT32
- exFAT
- NTFS
- ext2
- ext3
- ext4

## Technology Stack

| Component | Technology |
|---|---|
| Main application | Rust |
| GUI | FLTK / FLTK-RS |
| Recovery engine | PhotoRec |
| PhotoRec integration | C / Rust FFI |
| Disk imaging | `dd` |
| Partition management | GParted |
| NTFS support | ntfs-3g |
| Build system | Cargo |
| Development environment | Linux |

## Project Structure

A simplified view of the software architecture:

```text
Kaifuku
├── GUI Module
├── Device Detection
├── Disk Imaging
├── FFI Integration
├── Recovery Module
├── Binary Analysis
├── Advanced Mode
│   ├── Hex Editor
│   ├── Signature Scanner
│   ├── Header Template Generator
│   └── Recovery Log
└── Recovery Result Management
```

## Requirements

Kaifuku is intended to run on a 64-bit Linux environment.

Recommended hardware:

- 64-bit Intel or AMD processor
- 2 GB RAM or more
- Storage space sufficient for disk images and recovered files
- USB storage or other supported storage devices as recovery sources

When creating a disk image, the destination should preferably be stored on a **different storage device from the source**.

## Building

Make sure Rust and Cargo are installed.

Clone the repository:

```bash
git clone https://github.com/<your-username>/kaifuku.git
cd kaifuku
```

Build the project:

```bash
cargo build --release
```

Run the application:

```bash
cargo run --release
```

> The exact build and dependency requirements may vary depending on the current project configuration and PhotoRec integration.

## Important Considerations

### Do not recover files to the source device

Recovered data should be written to a separate storage device whenever possible.

Writing recovered data back to the source device may overwrite data that could otherwise be recovered.

### Disk Imaging

For important recovery operations, creating a disk image first is recommended.

```text
Original Storage
       │
       │  Disk Imaging
       ▼
   DD Image
       │
       ▼
    Recovery
```

This allows recovery operations to be repeated using the image rather than repeatedly accessing the original storage medium.

### Recovery Limitations

Data recovery is dependent on the condition of the storage device and the state of the underlying data.

Recovery may not be possible when:

- Data has already been overwritten
- The storage device has severe hardware failure
- The required data is no longer readable
- The underlying recovery engine cannot identify the required file data

Kaifuku uses PhotoRec's file-carving approach, so recovery results depend on the characteristics of the files and the available data on the storage device.

## Project Status

Kaifuku is an experimental data recovery and binary analysis project developed as a software engineering and research project.

The current focus is on:

- Data recovery workflow
- PhotoRec integration
- Rust–C FFI
- Disk imaging
- Binary analysis
- Recovery result management
- Advanced binary-analysis tools

## Future Development

Potential future improvements include:

- Support for additional file formats in binary analysis
- Support for additional file systems
- Improved storage-device diagnostics
- SMART information analysis
- More detailed recovery reports
- Additional binary-analysis tools
- Multilingual user interface
- Digital-forensics-oriented logging and reporting

## Disclaimer

Kaifuku is provided for legitimate data recovery, system administration, research, and educational purposes.

Always obtain appropriate authorization before analyzing or recovering data from storage devices that you do not own or have permission to access.

Data recovery is not guaranteed. Attempting recovery on damaged storage devices may cause further data loss, particularly if the device is unstable or failing.

## License

See the `LICENSE` file for the license applicable to this project and its components.

PhotoRec is developed by the CGSecurity project. Please refer to the applicable PhotoRec and CGSecurity licensing terms when using or redistributing modified PhotoRec components.

## Acknowledgements

- **PhotoRec / CGSecurity** — file-carving recovery engine
- **Rust** — application and binary-analysis development
- **FLTK** — graphical user interface
- **Alpine Linux / Linux ecosystem** — underlying tools and development environment

---

**Kaifuku — Data Recovery & Binary Analysis**