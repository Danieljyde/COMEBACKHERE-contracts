# Economic Parameters

## Treasury settlement batch cap

`MAX_BATCH_SIZE = 50` (`contracts/treasury/src/settlements.rs`) is the single shared cap on the
number of settlement IDs accepted per call for every settlement batch entrypoint:

- `batch_approve_settlements`
- `batch_cancel_settlements`
- `batch_execute_settlements`

Execution is the most instruction-expensive of the batch operations (it performs a token
transfer per settlement), so it must never use a looser cap than approval or cancellation.
Calls exceeding the cap panic with `BatchTooLarge`.
