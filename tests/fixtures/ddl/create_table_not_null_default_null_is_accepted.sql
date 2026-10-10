-- `CREATE TABLE t (c INT NOT NULL DEFAULT NULL)` is **accepted** on 18.6 (measured):
-- there are no rows to backfill, so the statement defines a NOT NULL column with no
-- default and the first insert that omits it is `23502`. The `CREATE TABLE`-side
-- control for the `ADD COLUMN` gate — a fix that refused the default outright rather
-- than judging the backfilled rows would break ordinary DDL here.
--
-- The `INSERT`s in the setup are the behavioural half: the omitting one would be the
-- statement 18.6 rejects, so only the explicit one runs, and the `@test` reads it back.
-- @setup
CREATE TABLE ctnd (id INT, c INT NOT NULL DEFAULT NULL);
INSERT INTO ctnd (id, c) VALUES (1, 5);
-- @test
SELECT id, c FROM ctnd ORDER BY id;
