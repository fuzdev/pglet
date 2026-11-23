-- `t*` is `relation_expr`'s "this table and its descendants" spelling; pglet has no
-- inheritance, so it names the table alone. `ALTER TABLE t* DROP CONSTRAINT` drops the
-- constraint, where pglet read the `*` into a tolerated tail and did nothing.
-- @setup
CREATE TABLE atsd (a INT CHECK (a > 0));

-- @test
ALTER TABLE atsd* DROP CONSTRAINT atsd_a_check;
INSERT INTO atsd VALUES (-1);
SELECT a, (SELECT count(*) FROM pg_constraint WHERE conname = 'atsd_a_check') AS checks
FROM atsd;
