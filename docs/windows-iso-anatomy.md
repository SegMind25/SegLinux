# How a Windows installation ISO works

This is the research Wincraft's design is based on: what is on a Windows 10/11
installation ISO, how it boots, and how Windows Setup turns it into an
installed system. [windows-to-wincraft.md](windows-to-wincraft.md) maps
each part onto its Linux/Artix counterpart.

Sources are Microsoft's deployment documentation on Microsoft Learn (listed
at the end). Where Microsoft's documents changed between Windows versions,
the current Windows 10/11 behaviour is described.

## 1. The image itself

A Windows ISO is a **UDF** file system (with a small ISO 9660 bridge), not
plain ISO 9660, because `install.wim` is usually larger than ISO 9660's 4 GiB
file size limit. It is built with `oscdimg`, which also writes two
**El Torito** boot catalog entries, one per firmware type:

```
oscdimg -m -o -u2 -udfver102 ^
  -bootdata:2#p0,e,b<src>\boot\etfsboot.com#pEF,e,b<src>\efi\microsoft\boot\efisys.bin ^
  <src> windows.iso
```

* `-bootdata:2`: two boot entries.
* `p0,e,b...etfsboot.com`: platform 0 (BIOS), no emulation, boot sector `etfsboot.com`.
* `pEF,e,b...efisys.bin`: platform 0xEF (UEFI), no emulation; `efisys.bin`
  is a small FAT image holding the UEFI boot loader.
* `-u2`: UDF only file system; `-o`: store duplicate files once.

### Layout

```
\
├── autorun.inf, setup.exe          run Setup from inside an existing Windows
├── bootmgr, bootmgr.efi            Windows Boot Manager
├── boot\
│   ├── etfsboot.com                BIOS El Torito boot sector
│   ├── bcd                         Boot Configuration Data store (BIOS)
│   ├── boot.sdi                    RAM-disk template for booting a WIM
│   ├── bootfix.bin                 "Press any key to boot from CD or DVD..."
│   └── fonts\, resources\
├── efi\
│   ├── boot\bootx64.efi            UEFI fallback loader (= Windows Boot Manager)
│   └── microsoft\boot\
│       ├── bcd                     BCD store (UEFI)
│       ├── efisys.bin              FAT image for the UEFI El Torito entry
│       └── efisys_noprompt.bin     same, without "press any key"
├── sources\
│   ├── boot.wim                    Windows PE + Windows Setup
│   ├── install.wim | install.esd   the operating system images (editions)
│   ├── setup.exe                   Windows Setup
│   ├── ei.cfg, pid.txt             optional: pin edition / product key
│   └── $OEM$\                      optional: files copied into the new system
└── support\
```

### WIM and ESD files

A **WIM** (Windows Imaging Format) file is a file-based, single-instance
compressed archive that can hold several *images*, addressed by index.
**ESD** is the same format with solid LZMS compression (smaller, slower,
read-only for most tools).

* `boot.wim` holds two images: index 1 is plain **Windows PE**, index 2 is
  Windows PE with **Windows Setup**, which is what the ISO boots.
* `install.wim`/`install.esd` holds one image per **edition** (Home, Home N,
  Pro, Education, ...). Setup asks which edition to install unless `ei.cfg`,
  `pid.txt`, a product key or the answer file decides.

`DISM` manages these images: `/Get-ImageInfo` lists indexes,
`/Apply-Image` extracts one onto a partition, `/Mount-Image` +
`/Add-Driver`/`/Add-Package` services an image offline, and
`/Capture-Image`/`/Export-Image` create and convert them.

## 2. Booting the ISO

**BIOS:** firmware → El Torito entry → `etfsboot.com` → `bootmgr` →
`\boot\bcd` → the BCD entry tells the boot manager to load `boot.wim`
(index 2) into a RAM disk described by `boot.sdi` → `winload` starts the
Windows PE kernel from the RAM disk → `setup.exe`.

**UEFI:** firmware → El Torito UEFI entry → `efisys.bin` (FAT) →
`\efi\boot\bootx64.efi` → `\efi\microsoft\boot\bcd` → the same RAM disk
boot of `boot.wim`.

On a USB stick (copied files, FAT32/NTFS) UEFI firmware simply runs
`\efi\boot\bootx64.efi`. All Microsoft boot files are signed, so the ISO
boots with **Secure Boot** enabled.

**Windows PE** is a minimal Windows that runs entirely from RAM. Besides
Setup it is used for deployment and recovery; **Windows RE**
(`winre.wim`, installed into a recovery partition) is a WinPE image with
repair tools.

## 3. Windows Setup and its configuration passes

Setup works in phases, and every setting in an answer file belongs to one
**configuration pass**:

| Pass | When it runs | Typical settings |
|------|--------------|------------------|
| **windowsPE** | In Windows PE, before anything is installed | UI language, disk partitioning (`DiskConfiguration`), which image to apply (`ImageInstall`), product key, EULA |
| **offlineServicing** | After the image is applied, before first boot | Drivers, updates and packages added to the offline image |
| **generalize** | Only with `sysprep /generalize` | Removes machine-specific data (SID, hardware state) so an image can be deployed to other PCs |
| **specialize** | First boot of the new system | Computer name, network and domain settings, `RunSynchronous` commands; unique SIDs are created |
| **auditSystem** / **auditUser** | Only in audit mode (OEM customisation) | Settings and commands before / after a user signs in |
| **oobeSystem** | Before Windows Welcome (OOBE) | User accounts, region, privacy, `FirstLogonCommands` |

In a normal installation the sequence is: windowsPE → apply image →
offlineServicing → reboot → specialize → oobeSystem/OOBE → desktop.

### What Setup actually does in windowsPE

1. Partition and format the disk (Setup's default layout, or the answer
   file's `DiskConfiguration`).
2. Apply the chosen image from `install.wim` to the Windows partition (the
   same operation as `DISM /Apply-Image`).
3. Create the boot environment with the equivalent of
   `bcdboot C:\Windows`: copy the boot files from `%WINDIR%\Boot` to the
   system (EFI) partition and create a BCD store from the BCD template;
   on UEFI also add a firmware boot entry.
4. Copy `$OEM$` content and the answer file into the new system, then reboot.

### Default disk layout

UEFI/GPT (current Microsoft recommendation):

| Partition | Purpose |
|-----------|---------|
| EFI system partition (ESP) | FAT32, boot manager and BCD; Microsoft documents 200 MB as the minimum |
| Microsoft Reserved (MSR) | 16 MB, no file system, for partition management |
| Windows | NTFS, the `C:` drive |
| Recovery | Windows RE tools (`winre.wim`) |

BIOS/MBR uses an active "System Reserved" partition instead of the ESP and
MSR.

## 4. Unattended installation

An **answer file** (XML, written with Windows System Image Manager) answers
Setup's questions in advance. Named `autounattend.xml`, Setup finds it
*implicitly*; the search order Microsoft documents is:

1. The registry value `HKLM\System\Setup!UnattendFile`
2. `%WINDIR%\Panther\Unattend`
3. `%WINDIR%\Panther` (Setup caches the file here for later passes)
4. Removable read/write media, in drive letter order (root folder)
5. Removable read-only media (DVD), in drive letter order (root folder)
6. `\Sources` of the installation media (windowsPE and offlineServicing passes)
7. `%WINDIR%\System32\Sysprep`
8. `%SYSTEMDRIVE%`

So putting `autounattend.xml` at the root of the USB stick makes Setup run
without questions. Settings that destroy data (such as
`<WillWipeDisk>true</WillWipeDisk>`) must be explicit.

Other customisation hooks:

* **`$OEM$` folders** (`sources\$OEM$`): `$OEM$\$$` is copied to
  `%WINDIR%`, `$OEM$\$1` to the system drive.
* **`SetupComplete.cmd`** in `%WINDIR%\Setup\Scripts\` runs with local
  system permissions at the end of Setup. It must not reboot the machine;
  its output is logged to `%WINDIR%\Panther\UnattendGC\setupact.log`.
  (`ErrorHandler.cmd` runs when Setup fails.)
* **`RunSynchronous`** (specialize/windowsPE) and **`FirstLogonCommands`**
  (oobeSystem) run commands from the answer file.

## 5. Logs and recovery

Setup logs to `setupact.log` (all actions) and `setuperr.log` (errors only)
in `Panther` folders: `X:\Windows\Panther` while in Windows PE,
`C:\$Windows.~BT\Sources\Panther` during the copy, and
`C:\Windows\Panther` once installed.

## 6. Requirements checks

Windows 11 Setup checks the hardware (TPM 2.0, Secure Boot capable UEFI,
4 GB RAM, 64 GB storage, supported CPU) before installing.

## Sources

* Microsoft Learn: [Windows Setup configuration passes](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/windows-setup-configuration-passes), [How configuration passes work](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/how-configuration-passes-work)
* Microsoft Learn: [Windows Setup automation overview](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/windows-setup-automation-overview) (implicit answer file search order)
* Microsoft Learn: [UEFI/GPT-based hard drive partitions](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/configure-uefigpt-based-hard-drive-partitions)
* Microsoft Learn: [BCDBoot command-line options](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/bcdboot-command-line-options-techref-di)
* Microsoft Learn: [Add a custom script to Windows Setup](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/add-a-custom-script-to-windows-setup) (SetupComplete.cmd)
* Microsoft Learn: [Create an ISO image for UEFI platforms](https://learn.microsoft.com/en-us/troubleshoot/windows-server/deployment/create-iso-image-for-uefi-platforms) (oscdimg `-bootdata`)
* Microsoft Learn: DISM image management and WinPE documentation
