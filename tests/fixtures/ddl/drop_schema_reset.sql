-- The migration/test reset idiom: `DROP SCHEMA public CASCADE; CREATE SCHEMA
-- public` clears the whole catalog, then the schema is rebuilt and queried.
-- fuz_app's test harness (`testing/db.ts`'s `reset_pglite`) issues this between
-- tests. The drop+recreate cycle completes and the rebuilt table reads back.
-- @setup
CREATE TABLE old_table (id INT, junk TEXT);
INSERT INTO old_table VALUES (1, 'stale');
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
CREATE TABLE new_table (id INT, label TEXT);
INSERT INTO new_table VALUES (1, 'fresh');
-- @test
SELECT id, label FROM new_table ORDER BY id;
