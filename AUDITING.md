# How a record is found eligible, and what destruction does

This document is for an auditor or counsel asking how a record came to be
destroyed under Slipcase Fileroom or the `fileroom` command. It describes what the `fileroom` crate does, which
is the only code in the product that destroys anything. The normative rules
are the [Slipcase Records Profile](https://github.com/excelano/slipcase-profiles/tree/main/records);
where this page and the specification differ, the specification is right
and the code has a defect.

## What a record is

A record is a Slipcase container: a ZIP file holding one content file and a
TOML flyleaf describing it, with the profile's additional members beside
them. Everything the record says about itself is in that one file. Its
flyleaf names the retention series it is under, the holds applied to it,
its custodian, and a hash of its content; its event log, a member of the
same container, records every change to those facts as a hash-chained
sequence of entries that cannot be edited without the edit showing. There
is no central database that could disagree with the file.

## Eligibility

Eligibility is a function of the record, the retention schedule in force,
the organization's settings, the active hold matters, and an evaluation
date that is always stated. Nothing in the computation reads a clock.

For each series the record is under, the schedule gives a period and says
what the period is measured from: the record's creation date, or an event
the organization recorded, such as the closing of the matter the record
belongs to. A cutoff, where the series has one, moves the start of
retention to the end of the calendar year, fiscal year, quarter, or month
containing the trigger. The due date is that start plus the period, with
months and years landing on the same day of the month or, where it does
not exist, the last day. A series is due when the evaluation date is on
or after its due date.

A record is eligible when, and only when, it has at least one series and
every series is due with the action destroy; no series is permanent or
calls for retention or transfer; no review series is due and undecided; the
record carries no hold and no active matter's scope matches it; and no
flag blocks it. A record under several series is eligible only under all of
them, so the longest retention governs. Where a series has a maximum as
well as a minimum, passing the maximum raises a flag, and a hold still
wins: a held record past its maximum is not eligible.

Anything short of that is one of three other outcomes. Not eligible, with
every reason that applies: permanent, retained, held, awaiting its event,
period not elapsed, no series, or a retired series. Review due, when a
review series has fallen due and nobody has decided it. Cannot evaluate,
when the record or a series it is under cannot be read well enough to say:
a series the schedule does not carry, a series whose row states no
computable rule, a container that cannot be read, a version of Slipcase or
of the profile this build does not implement, or a profile table that
breaks a rule. None of those four outcomes but the first permits
destruction.

The schedule is a versioned store in the records root. A version is never
edited; a change is a new version. Every plan records the version it was
evaluated against, and so does every batch in the register, so any
decision traces to the exact schedule in force when it was made.

## Planning and approval

A plan is the list of records found eligible on a stated date against a
stated schedule version. It records each record's identifier, path, series,
custodian, creation date, the hash of its content, and the hash of its
flyleaf as it was read. A plan nobody has approved is not executed.
Approval is a statement made to the implementation, which records it and
does not verify it; who may approve belongs to the organization, not to
this software.

## Destruction

A run claims the next sequence number in the disposition register by
creating the batch's intent file with create-new semantics, so that two
runs can never hold one number, and only when the previous batch is final.
The intent carries the plan, outcomes blank. Then, for each record in plan
order, the run reads the container again and checks three things before
touching it: that its flyleaf still hashes to the value in the plan, so a
record changed since approval is skipped; that it is still eligible as of
the plan's evaluation date, with every active hold matter's scope evaluated
against it, so a hold placed since planning is honoured; and only then
does it delete the file and confirm the file is gone. Each outcome is
appended to the batch's journal, itself a hash-chained log, before the
next record is touched. Deletion is an ordinary file system delete.
Nothing is overwritten, and nothing written under this profile claims
secure erasure or media sanitization. The scope statement in every batch
says what removal from the repository means for the organization,
including what happens to backups.

When every record has an outcome, the run writes the batch's final
container, again by create-new, holding the completed manifest, the
certificate, and five hashes: the intent's flyleaf, the previous final's
flyleaf, the certificate, the manifest, and the journal. The chain begins
at a published genesis value. Verification walks the register from the
first batch and reports the first point at which any hash, any sequence
number, or any intent fails to match; it repairs nothing.

A run that dies leaves an intent without a final. No run finishes it
automatically, because nothing in the register can tell a crashed run from
one still executing on another machine. A person confirms the run is dead,
and recovery then finishes the batch from the journal and from what is
present: journaled outcomes stand, a planned record still present is
recorded as skipped and not attempted, and one gone with no journal entry
is recorded as unknown. Recovery destroys nothing, and the final it writes
says it was a recovery, by whom, and when.

## What survives

After destruction, the record's row in the batch's manifest is the only
evidence it existed: its identifier, its title and path, its series, its
custodian, its creation date, the hash of its content, and its outcome.
The certificate restates those facts in words. Both are held in the register
under the chain described above, so an edit to either, or the removal of a
batch, is detectable by anyone with the files and this crate's `verify`
command, with no license and no vendor.
