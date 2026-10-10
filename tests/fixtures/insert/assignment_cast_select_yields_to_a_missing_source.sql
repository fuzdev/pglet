-- **An error-order control — do not trim it.** Postgres analyses the source query before
-- it coerces anything into the target's columns, so a source relation that does not
-- exist is `42P01` and nothing else. pglet raises that at **execution**, which is what
-- makes a plan-time assignment gate able to pre-empt it — and it would, with a type
-- nothing in the statement has: `Plan::output_columns` substitutes `text` for a column
-- of a relation it cannot find, so this answered `42804 … is of type integer but
-- expression is of type text` until the gate learned to decline an unresolvable source.
-- @setup
CREATE TABLE acms (id INT, i INT);

-- @expect-error: relation "acms_nosuch" does not exist (42P01)
-- @test
INSERT INTO acms (i) SELECT n FROM acms_nosuch;
