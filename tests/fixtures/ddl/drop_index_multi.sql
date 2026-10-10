-- `DROP INDEX a, b` removes **both**, and the table and its rows are untouched.
--
-- **The surviving-index count is the observable, and it has to be.** The obvious
-- assertion — a `SELECT` from the table afterwards — answers identically whether one
-- index went or both, because an index changes only speed and never a result (§ Build
-- profiles). Measured: with phase 3 mutated to drop only the first name
-- (`index_oids.iter().take(1)`), a row-selecting version of this fixture stayed green
-- along with the entire 1638-fixture corpus and the whole unit suite — the multi-name
-- `DROP INDEX` success path was pinned by nothing at all. Its sibling kinds were already
-- covered (`sequences/drop_sequence_multi.sql` reclaims both names,
-- `ddl/drop_table_multi_fk_cycle.sql` asserts both tables gone); only this one was not.
--
-- `pg_indexes` is the direct reading and agrees name-for-name on both engines here. The
-- refusal spellings live in the three `drop_index_multi_*_loud.sql` siblings.
-- @setup
CREATE TABLE dixm (a INT, b INT);
INSERT INTO dixm VALUES (1, 1), (2, 2);
CREATE INDEX dixm_a ON dixm (a);
CREATE INDEX dixm_b ON dixm (b);

-- @profile-lock
-- @test
DROP INDEX dixm_a, dixm_b;
SELECT count(*)::int AS surviving FROM pg_indexes WHERE tablename = 'dixm';
