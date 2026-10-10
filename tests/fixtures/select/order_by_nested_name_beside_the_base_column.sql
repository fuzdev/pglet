-- The base column the nested key binds to is itself projected, under another name, so
-- the two candidate bindings are both present in the output row and the wrong one is
-- not merely a different value but a different *column of this very result*.
-- `ORDER BY q+0` is the base `q` (3, 1, 2), so the rows come out `200|1 300|2 100|3`;
-- binding the nested `q` to the output alias sorts by `a` instead and answers
-- `100|3 200|1 300|2`.
--
-- Its duplicate-name spelling — `SELECT a AS q, q FROM onbb ORDER BY q+0`, two output
-- columns both named `q` — is the shape 18.6 answers and pglet raised `42702` on, since
-- the sort-above resolver counts output names rather than comparing target entries. It
-- carries no fixture because the harness keys rows by column name (`EDGES.md` row 35,
-- § Harness limits); the `AS r` spelling here discriminates the same binding.
-- @setup
CREATE TABLE onbb (q INT, a INT);
INSERT INTO onbb VALUES (3, 100), (1, 200), (2, 300);

-- @test
SELECT a AS q, q AS r FROM onbb ORDER BY q+0;
