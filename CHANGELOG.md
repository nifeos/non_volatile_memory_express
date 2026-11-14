# Changelog

Dates are UTC. One line per release, what changed and why, so a reader of the diff has the
sentence beside it.

## 0.1.0-pre.2

- Each decoded field has its specification mnemonic as an alias, and the transfer unit travels
  with `Namespace`. The memory page size is an argument of the enabling configuration, and
  `maximum_data_transfer_size_shift` ends in its unit.
- New: Identify Controller parsing, the completion status by its parts and the phase before the
  entry (the phase readers are renamed for the question each answers), an Admin Queue Attributes
  builder, the clean shutdown value and Shutdown Status bits, command identifiers tracked in
  flight, and the page list a transfer of three or more pages needs.
- A software controller example runs the whole loop.

## 0.1.0-pre.1

- First version.

## BUGS

- The template's lag check guards the CI pin, not this file; a change that matters to a user of
  the crate belongs in a release line here.
