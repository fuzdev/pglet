-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- The other half of the junk-name collision, and the one an unguessable spelling
-- cannot close: a **later key** naming the generated column. A key the pre-junk
-- output cannot resolve is left for the executor to judge, and by then the junk
-- column is on the row — so this answered three rows where 18.6 is `42703`. The
-- generated name is reserved against every name the clause's own keys mention, not
-- only against the output columns, which is why the first key's junk column is
-- `pglet_junk_1` here.
-- @expect-error: 42703 — a key naming the generated junk column resolves to nothing
-- @test
SELECT count(*) FROM gj GROUP BY k ORDER BY k, pglet_junk_0;
