-- A stored `CHECK (b IS TRUE)` — the persisted half of the boolean-test family.
-- Both `false` and NULL violate it, which is exactly what separates `IS TRUE` from
-- the bare predicate `CHECK (b)` (a NULL predicate passes a CHECK on both engines).
-- The value fixture `check_is_true_accepts_true.sql` runs the accepting row.
-- @setup
CREATE TABLE tk (id INT, b BOOL, CHECK (b IS TRUE));
INSERT INTO tk VALUES (1, true);
-- @test
-- @expect-error: check constraint violated by a NULL
INSERT INTO tk VALUES (2, NULL);
