-- The BUG-2 x BUG-3 interaction: a DEFAULT whose backfilled value violates the new
-- column's own inline CHECK aborts loud (23514) — the case the old NULL-punt hid.
-- @expect-error: backfilled default violates the new CHECK
-- @setup
CREATE TABLE alter_addcol_ckd (id INT);
INSERT INTO alter_addcol_ckd VALUES (1);

-- @test
ALTER TABLE alter_addcol_ckd ADD COLUMN val INT CHECK (val > 10) DEFAULT 5;
