-- The floor of the field set: `25P02` is `S`/`V`/`C`/`M` and nothing else on both
-- engines. Its value here is the negative — pglet must not start inventing `s`/`t` for
-- an error 18.6 leaves bare.
-- @no-txn
-- @setup
BEGIN;
SELECT 1/0;

-- @test
-- @expect-error: the block is aborted
-- @expect-message
SELECT 1;
