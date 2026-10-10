-- A set operation's two branches unify to the enum **either** side declares: 18.6 types
-- an `unknown` literal branch by the other branch's type, so this column is `mood` and
-- orders by member position. Reading a branch that declares no enum as a *veto* left it
-- on the label bytes — the exact reverse for `mood` — and that is the pair real SQL
-- produces, since a branch that is itself an enum column is the rarer spelling.
--
-- The `AS lbl` is load-bearing (`docs/enums.md` § Harness note), and the `id` tiebreak
-- is too: the two `ok` rows are `Equal` under the member order, so without it the row
-- order between them is unspecified.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, x::text AS lbl
FROM (SELECT id, m AS x FROM t UNION ALL SELECT 0, 'ok') q
ORDER BY x, id;
