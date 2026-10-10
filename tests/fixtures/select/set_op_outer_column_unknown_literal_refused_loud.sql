-- The constant converts to the enclosing column's `integer`, so a non-numeric one is
-- refused rather than compared as text.
-- @setup
CREATE TABLE sul (i INT);
INSERT INTO sul VALUES (7);
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT count(*) FROM sul WHERE EXISTS (SELECT i EXCEPT SELECT 'x');
