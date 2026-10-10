-- @no-txn
-- Runs with no transaction open, so it opts out of the harness's isolating
-- wrapper.
--
-- The `ROLLBACK TO` member of the `25P01` family. Worth its own fixture rather
-- than being folded into the `SAVEPOINT` one: this spelling leads with the
-- `ROLLBACK` keyword, so it travels a different route through the parser and
-- through the aborted-block gate, and a `25P01` here is not implied by a `25P01`
-- there.
-- @setup
SELECT 1;

-- @expect-error: ROLLBACK TO SAVEPOINT can only be used in transaction blocks (25P01)
-- @test
ROLLBACK TO s;
