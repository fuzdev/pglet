-- @no-txn
-- A rename taken back with its block: the old name reads the table again, the rows the
-- block wrote under the new name are gone, and the new name names nothing.
-- @setup
DROP TABLE IF EXISTS rc_rb, rp_rb, rq_rb;
CREATE TABLE rp_rb (id serial PRIMARY KEY, v text UNIQUE);
INSERT INTO rp_rb (v) VALUES ('a'), ('b');
BEGIN;
ALTER TABLE rp_rb RENAME TO rq_rb;
INSERT INTO rq_rb (v) VALUES ('c');
ROLLBACK;
-- @test
SELECT (SELECT count(*) FROM rp_rb) AS rows_kept, to_regclass('rq_rb') IS NULL AS new_name_gone;
