-- One `ON CONFLICT DO UPDATE` statement whose row source proposes one key twice meets, on the
-- second proposal, the row its first proposal updated. 18.6 refuses the statement `21000`
-- where it locks that row (`ExecOnConflictUpdate`, the PostgreSQL source's
-- `ExecOnConflictLockRow`: the current command wrote it) and nothing
-- the statement did stands; pglet used to run the second action against the first's write.
--
-- Here the first proposal **updates** a row that was already there, so a refusal that fires
-- only for a row the statement *inserted* is red. Its twin, where the first proposal
-- inserts, is `on_conflict_do_update_same_row_twice_first_inserts_loud.sql`.
-- @profile-lock
-- @setup
CREATE TABLE ocst (id INT PRIMARY KEY, a INT, b INT);
INSERT INTO ocst VALUES (1, 10, 0);

-- @test
-- @expect-error: ON CONFLICT DO UPDATE command cannot affect row a second time (21000)
-- @expect-message
INSERT INTO ocst VALUES (1, 0, 0), (1, 0, 0)
ON CONFLICT (id) DO UPDATE SET a = ocst.a + 1, b = ocst.a
RETURNING *;
