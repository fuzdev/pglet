-- A qualifier a level owns binds a column of that relation, and what binds the value binds
-- its declared type, an aggregate's operand check and `MIN` / `MAX`'s enum order too: the
-- aggregate and window nodes describe their output through the resolver they evaluate
-- with. Over `dt2 ("o.n" TEXT, n INT)` and `dt3 ("o.m" TEXT, m dt_mood)` under the alias
-- `o`, `o.n` and `o.m` are the columns `n` and `m`. A descriptor scope rebuilt from the
-- bare names bound them by their `"o.n"` / `"o.m"` text instead: `max(o.n)` answered `4`
-- declared `text`, `sum(o.n)` was `42883 sum(text)`, and `max(o.m)` / `min(o.m)` ordered
-- the enum by its label bytes.
--
-- This one: an aggregate's declared type and operand check.
-- @setup
CREATE TYPE dt_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE dt2 ("o.n" TEXT, n INT);
CREATE TABLE dt3 ("o.m" TEXT, m dt_mood);
INSERT INTO dt2 VALUES ('x', 3), ('y', 4);
INSERT INTO dt3 VALUES ('a', 'happy'), ('b', 'sad'), ('c', 'ok');

-- @test
SELECT max(o.n) AS mx, sum(o.n) AS sm FROM dt2 o;
