-- The control for `add_column_default_sequence_expression_is_not_cooked_eagerly`: the
-- same expression, over a table that *has* rows. The backfill draws per row, the first
-- draw makes the divisor zero, and both engines answer `22012` — so the exemption
-- defers the verdict rather than discarding it, and a green pair identifies the cook's
-- probe rather than a default that simply never errors.
-- @setup
CREATE TABLE addcol_seqdiv (id INT);
INSERT INTO addcol_seqdiv VALUES (1), (2);
CREATE SEQUENCE addcol_seqdiv_q;
-- @expect-error: the backfill's first row divides by zero
-- @test
ALTER TABLE addcol_seqdiv ADD COLUMN a INT DEFAULT 100 / (nextval('addcol_seqdiv_q') - 1);
