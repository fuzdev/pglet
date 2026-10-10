-- The static comparison gate must still fire over a scope whose relations share a
-- column name. It used to be switched off wholesale for any such scope — a stopgap
-- that kept it from disagreeing with the then-bare-name-keyed coercion pass — so two
-- relations sharing even a harmless type-identical `id` disabled the gate for **every**
-- comparison in the SELECT, and `WHERE flag = 5` returned zero rows instead of the
-- `42883` Postgres raises. (Renaming `jsg_q.id` was enough to make the same predicate
-- error correctly.) Both passes now resolve through one qualifier-aware scope, so the
-- gate needs no such opt-out.
-- @setup
CREATE TABLE jsg_p (id INT, flag BOOL);
CREATE TABLE jsg_q (id INT);
INSERT INTO jsg_p VALUES (1, true);
-- @expect-error: boolean = integer has no operator (42883)
-- @test
SELECT * FROM jsg_p JOIN jsg_q ON jsg_p.id = jsg_q.id WHERE flag = 5;
