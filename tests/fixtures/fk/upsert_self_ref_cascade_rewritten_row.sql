-- @no-txn
-- Its own statements write the rows, for the reason `update_self_ref_cascade_rewritten_row_referencing_row_first.sql`
-- gives.
--
-- A row an `ON CONFLICT DO UPDATE` rewrites that still references a key a later rewritten
-- row moves is repointed by that row's cascade.
-- @setup
DROP TABLE IF EXISTS sr_upsert;
CREATE TABLE sr_upsert (id int PRIMARY KEY, par int REFERENCES sr_upsert ON UPDATE CASCADE);
INSERT INTO sr_upsert VALUES (1, NULL), (3, NULL), (2, 1);
UPDATE sr_upsert SET par = 2 WHERE id = 3;
-- @test
-- @profile-lock
INSERT INTO sr_upsert VALUES (3, 0), (2, 0) ON CONFLICT (id) DO UPDATE SET id = sr_upsert.id + 10;
SELECT id, par FROM sr_upsert ORDER BY id;
