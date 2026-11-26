# Changelog

Dates are UTC. One line per release, what changed and why, so a reader of the diff has the
sentence beside it.

## Unreleased

- `parse_identify_namespace` refuses a namespace whose logical block address (LBA) format in use carries metadata, which the
  crate does not support. It accepted one as if each block held only its data, so a caller sized
  transfer buffers too small. New: `metadata_size_bytes` reports the metadata size a refusal saw.

## 0.1.0-pre.4

- The crate's code does not change. The QEMU driver example runs at queue depth 2 and with a 16 KiB
  page, and checks every reported head.
- The README states under BUGS that the crate does not support completion by interrupt yet and does
  not read the queue entry sizes that Identify Controller reports.

## 0.1.0-pre.3

- Names follow the architect's rulings, with no alias for the old spellings. `InFlight` is
  `CommandIdentifierSet`, and its `mint` is `acquire`. `Completion::command_retry_delay` is
  `command_retry_delay_selector`, `is_more_set` is `has_more_status_information`, and
  `is_do_not_retry_set` is `is_retry_refused`.
- `IdentifyController::maximum_data_transfer_size_shift` is `maximum_data_transfer_page_shift`,
  a count of pages of the controller's minimum page size. `IdentifyNamespace::blocks` is
  `size_in_logical_blocks`, and `shutdown_controller_configuration` is
  `controller_configuration_with_shutdown_notification`.
- The specification mnemonics stay as documentation aliases.

## 0.1.0-pre.2

- Each decoded field has its specification mnemonic as an alias, and the transfer unit travels
  with `Namespace`. The memory page size is an argument of the enabling configuration, and
  `maximum_data_transfer_size_shift` ends in its unit.
- New: Identify Controller parsing, and the completion status read by its parts. The phase
  readers take their names from the question each answers.
- New: an Admin Queue Attributes builder, the clean shutdown value and Shutdown Status bits, and
  command identifiers tracked in flight.
- New: the page list a transfer of three or more pages needs.
- A software controller example runs the whole loop.

## 0.1.0-pre.1

- First version.

## BUGS

- The template's lag check guards the CI pin, not this file; a change that matters to a user of
  the crate belongs in a release line here.
