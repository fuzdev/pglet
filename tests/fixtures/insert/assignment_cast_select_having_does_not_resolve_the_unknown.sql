-- **A false-rejection control — do not trim it.** A `HAVING` is a filter over the
-- grouped rows and names no target-list entry, so the `'t'` here stays Postgres's
-- `unknown` and the `BOOL` column resolves it. Accepted on 18.6.
--
-- Its refuse twin is `assignment_cast_select_distinct_resolves_the_unknown_loud.sql`.
-- Both lower to a plan the immediate-node rule reads as "not a projection", which
-- refused this statement.
-- @setup
CREATE TABLE ache (id INT, b BOOL);
CREATE TABLE achesrc (n INT);
INSERT INTO achesrc VALUES (1);
-- @test
INSERT INTO ache (b) SELECT 't' FROM achesrc HAVING count(*) > 0 RETURNING b;
