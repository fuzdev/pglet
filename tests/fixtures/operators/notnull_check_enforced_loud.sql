-- A `CHECK (a NOTNULL)` refuses a NULL.
-- @setup
CREATE TABLE nce (a INT CHECK (a NOTNULL));
-- @test
-- @expect-error: violates check constraint (23514)
INSERT INTO nce VALUES (NULL);
