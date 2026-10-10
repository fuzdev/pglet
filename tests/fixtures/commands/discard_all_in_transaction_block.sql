-- `PreventInTransactionBlock`: `DISCARD ALL` is `25001` inside a block on 18.6, and
-- the harness's own isolating `BEGIN … ROLLBACK` is that block, so this fixture
-- needs no transaction statements of its own. The SQLSTATE is compared against the
-- oracle's, so a pglet that merely refused with some other code would fail.
--
-- Only `ALL` is guarded — `DISCARD PLANS`, `SEQUENCES` and `TEMP` all run inside a
-- block on 18.6, measured.
-- @test
-- @expect-error: DISCARD ALL cannot run inside a transaction block
DISCARD ALL;
