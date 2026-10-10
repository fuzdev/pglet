-- An all-unknown set operation resolved its column to `text` before the target column
-- sees it, so the assignment into `integer` is `42804`.
-- @setup
CREATE TABLE ssa (i INT);
-- @expect-error: column "i" is of type integer but expression is of type text
-- @test
INSERT INTO ssa (i) SELECT '3' UNION SELECT '4';
