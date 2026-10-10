-- The accept side of the `search_path` gate. Every element here resolves to the one
-- schema on both engines — `"$user"` names a schema a fresh database does not have
-- and `pg_catalog` is searched implicitly — so the statement is accepted and the
-- unqualified read that follows still answers.
--
-- Refusing this list is the plausible wrong implementation the gate invites, and it
-- is the spelling a driver actually sends: `"$user", public` is Postgres's own
-- default, echoed back by a pool that captured it with `SHOW search_path`.
-- @setup
CREATE TABLE sp_rows (n INT);
INSERT INTO sp_rows VALUES (1);
-- @test
SET search_path TO "$user", public;
SELECT n FROM sp_rows;
