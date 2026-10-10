-- Same gap on the `SET` right-hand side: the assignment was coerced to the target
-- column's type without the expression ever being checked, so this updated a row.
-- @setup
CREATE TABLE usc (i4 INT4, tx TEXT);
INSERT INTO usc VALUES (1, 'a');
-- @test
-- @expect-error: COALESCE branches have no common type (42804)
UPDATE usc SET tx = COALESCE(i4, tx);
