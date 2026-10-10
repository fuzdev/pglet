-- An array of the target type is still not the target type: 18.6 names
-- `boolean[]`, and so does this.
-- @setup
CREATE TABLE bcn5 (ba BOOL[]);
INSERT INTO bcn5 VALUES ('{true}');
-- @expect-error: argument of WHERE must be type boolean, not type boolean[]
-- @test
SELECT count(*) FROM bcn5 WHERE ba;
