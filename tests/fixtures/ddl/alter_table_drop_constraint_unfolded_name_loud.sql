-- A constraint declared under a quoted mixed-case name is not found by its unquoted
-- spelling, which folds to lower case.
-- @expect-error: the folded name names no constraint
-- @expect-message
-- @setup
CREATE TABLE dcun (x INT CONSTRAINT "Dcun_Upper" CHECK (x > 0));

-- @test
ALTER TABLE dcun DROP CONSTRAINT Dcun_Upper;
