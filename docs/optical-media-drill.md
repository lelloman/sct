# Physical optical-media drill

This is the release procedure for producing and qualifying physical copies of
ScT archive images. It is deliberately separate from the Docker ISO tests in
[extensive qualification](extensive-qualification.md): an ISO that can be
extracted is not evidence that a drive wrote readable media.

The supported offline recovery target is Linux x86_64 with glibc 2.36 or newer.
It was qualified on Debian 12. The disc itself needs no ScT service, database,
network, credentials, compiler, package installation, or browser. It is not a
bootable operating system. Windows, macOS, other CPU architectures, automatic
burner control and parity repair are outside this release.

## Release artifacts before a burn

An administrator creates a collection and freezes a plan in the browser, then
waits until every disc in the set is **Ready**. The collection timeline and disc
page expose the immutable ISO download, SHA-256 and byte length. Download every
image before its cache deadline and validate it locally; an expired image is not
promised to be regenerable.

```sh
sha256sum /secure/downloads/disc-001.iso
stat -c '%s bytes' /secure/downloads/disc-001.iso
```

Compare both values to the disc detail shown by ScT. Keep the downloaded ISO
until the physical read-back has succeeded. The browser only plans, downloads and
records physical-copy observations; it neither burns media nor changes snapshot
content.

Before a release, build and test the exact recovery bundle that the export host
will install on each image:

```sh
./scripts/build-recovery-bundle /absolute/new/recovery-bundle release-YYYYMMDD
./scripts/test-recovery-bundle /absolute/new/recovery-bundle
```

The second command runs the exact packaged executable in read-only, networkless
Debian. Record the manifest digest and byte count printed by the builder, the
Git revision, and the tested writer version. Never replace the bundle in a
frozen plan or published ISO.

## Select and inspect the writer

Use a supported Linux host with `xorriso` and a connected optical writer. Do not
guess the device path. List it immediately before the burn and record the output:

```sh
xorriso -version
xorriso -devices
```

Confirm the selected device, medium type and blank/appendable state with the
operator. The following examples use `/dev/sr0` only as a placeholder. Replace
it with the device returned by `xorriso -devices`; it is never an instruction to
write that path blindly. A `-dummy` run is useful for checking drive access but
does not qualify media:

```sh
xorriso -as cdrecord dev=/dev/sr0 -dummy -v -eject -isosize /secure/downloads/disc-001.iso
```

For a confirmed blank BD-R or other supported writable medium, write exactly one
immutable ISO:

```sh
xorriso -as cdrecord dev=/dev/sr0 -v -eject -isosize /secure/downloads/disc-001.iso
```

Do not use multisession/append commands for ScT images. A burn failure leaves the
logical disc and all historical dependencies intact. Start again with another
physical copy of the same ISO, label it as a new physical copy in ScT, and do not
overwrite, retire or renumber a logical disc.

## USB and other removable filesystems

The media capacity in a collection is an explicit byte value; it is not limited
to Blu-ray sizes. Select the usable capacity of a USB stick, removable SSD, or
other mounted filesystem when planning the collection. The export remains an
immutable ISO so the same verified artifact can be burned to optical media or
materialized onto a filesystem. The recovery layout on the mounted filesystem is
identical to the ISO layout.

Download and verify the ISO hash shown in ScT, mount the **empty** destination
filesystem yourself, then run the supplied materializer. It never formats,
mounts, discovers, or chooses a block device, and refuses a nonempty destination.

```sh
./scripts/materialize-archive-media \
  --sha256 "$ISO_SHA256_FROM_SCT" \
  /secure/downloads/disc-001.iso /media/sct-usb-001
```

The materializer extracts the ISO with `xorriso`, copies the on-media recovery
binary to private temporary storage, and runs its full media verification against
the USB filesystem. Keep the terminal output as the initial materialization
record, then run the read-back report below after safely ejecting and remounting
the device. A USB copy is still a physical copy of the same logical disc; label
its medium and location in ScT. It cannot make a larger device create extra data
for an already frozen plan, and it must not be mixed with unrelated files.

## Read back the actual disc

After the drive has finished and the medium is mounted read-only, inspect the
mount source and copy the executable to private writable storage in case the
mount is `noexec`:

```sh
findmnt -no SOURCE,TARGET /media/scT-disc-001
mkdir -m 700 /private/sct-readback
cp /media/scT-disc-001/recovery/sct-recover /private/sct-readback/recover
chmod 700 /private/sct-readback/recover
/private/sct-readback/recover inspect /media/scT-disc-001 /private/sct-readback/index.sqlite
/private/sct-readback/recover verify-report /media/scT-disc-001 /private/sct-readback/original.sqlite > /private/sct-readback/disc-report.json
```

`verify-report` must exit 0. It verifies the original inventory for that logical
disc; do not reuse a later set's work index for this command. Keep the JSON report,
writer/drive/media identifiers, ISO SHA-256, label, location, dates and command
output. In ScT's **Physical copies** view, create the copy label/location and
submit the report using the copy's current revision. An exit of 2, any I/O error,
or a report from an extracted ISO is not a read-back verification. Record the
copy as damaged or unknown as appropriate; do not claim it is verified.

## Recover from a non-final disc

This is the mandatory recovery part of every physical drill. Pick a disc which is
not the final display number of its set, mount it read-only, and start from that
disc. Each disc contains the complete index/map and recovery executable, so no
final disc, service, database or internet connection is needed to enumerate the
target snapshot.

```sh
mkdir -p -m 700 /private/sct-restore/cache /private/sct-restore/destination
cp /media/scT-disc-002/recovery/sct-recover /private/sct-restore/recover
chmod 700 /private/sct-restore/recover
/private/sct-restore/recover ingest /media/scT-disc-002 /private/sct-restore/set.sqlite /private/sct-restore/cache
# Swap and ingest every required surviving disc in any order.
/private/sct-restore/recover ingest /media/scT-disc-001 /private/sct-restore/set.sqlite /private/sct-restore/cache
/private/sct-restore/recover restore /private/sct-restore/set.sqlite /private/sct-restore/cache /private/sct-restore/destination
```

Compare the resulting path set and bytes against the frozen source manifest or
independent SHA-256 inventory captured before the drill. To demonstrate a missing
disc, omit it and run `restore ... --partial`: exit 2 and JSONL missing-file/extent
records are required. Only files with every extent verified may be written. Never
present that output as an application-consistent snapshot, and never silently use
an older file version.

For historical discovery without ScT, scan a representative actual disc from each
set into a separate local catalog:

```sh
/private/sct-restore/recover catalog-scan /media/scT-disc-002 /private/sct-restore/catalog.sqlite /private/sct-restore/set.sqlite
/private/sct-restore/recover catalog-inspect /private/sct-restore/catalog.sqlite
```

Use a distinct work index for each target snapshot. The catalog preserves known
historical indexes but cannot prove that no later sets exist or recover accounts,
grants, credentials or physical-copy observations.

## Capacity, retention and operational limits

The chosen media budget includes filesystem overhead, the replicated full index,
the full recovery bundle and payload. Planning fails explicitly if that repeated
metadata cannot fit a selected medium; an ISO is never abbreviated into a chain
of prior indexes. The writer rechecks actual ISO size and rejects an oversized
image before publication. See [format limits](disc-archive-format.md#limits-and-compatibility)
and [export reservations](disc-archive-exports.md).

Initial limits are four active sets, 1 TiB pinned logical source bytes, 100 million
capture records per set, a 24-hour capture/export deadline, and 100 GiB of account
image cache. Building reserves all planned images plus twice the largest disc
budget for staging and extraction. Capacity or metadata-limit failure leaves a
visible failed/incomplete job; it does not publish a partial image or release
source pins prematurely. Cancellation, expiry, authority revocation and service
restart fence workers before releasing pins/reservations. Cached ISO expiry only
removes the cache artifact after leases end: it never deletes logical discs,
indexes, recovery points or physical-copy history.

There is no disc retirement policy. An older disc that is irrelevant to the latest
set can still be required for an older recovery point and must remain catalogued.

## Qualification record

| Field | Record |
| --- | --- |
| Procedure version | 2026-09-22 / archive format 1 |
| Automated ISO and offline-bundle evidence | [Docker archive stage](extensive-qualification.md#archive-recovery-stage) and `scripts/test-recovery-bundle`; these are not physical burns |
| Writer available on this qualification host | `xorriso` 1.5.6 |
| Drive discovery on 2026-09-22 | `xorriso -devices` reported `No drives found`; `/dev/sr*` and `/dev/cdrom` were absent |
| Physical burn/read-back/non-final-disc restoration | **Not performed:** no writable optical drive or media was available |
| Remaining release blocker | Perform this exact drill with a discovered writer and recorded writable media, then attach the successful actual-media `verify-report` and independent non-final-disc restore evidence. Do not mark hardware support qualified before that record exists. |
