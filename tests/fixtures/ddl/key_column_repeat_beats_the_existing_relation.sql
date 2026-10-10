-- The third of the three, and the one that outranks the most: `transformIndexConstraint`
-- runs in parse analysis, so the repeat beats each of the execution-time verdicts
-- measured against it on this route — the relation's `42P07`, which this fixture holds,
-- and the duplicate column, the system column name, the column-default cook and a
-- colliding declared `SEQUENCE NAME` beside it. `EDGES.md` row 39 carries the
-- arrangement at both routes, including what the repeat loses to; which part of it the
-- corpus also pins is the corpus's to answer, since a coverage list written here goes
-- stale the next time a fixture is added and reddens nothing.
-- @setup
CREATE TABLE kcber (q INT);
-- @expect-error: the repeated key column outranks the existing relation
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE kcber (b INT, PRIMARY KEY (b, b));
