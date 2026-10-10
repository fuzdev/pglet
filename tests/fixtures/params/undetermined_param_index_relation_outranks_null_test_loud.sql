-- An index's name is in the relation namespace and is not a relation a query can read:
-- 18.6 refuses it as it resolves the range table, ahead of the parameter. The null test
-- is ahead of the comparison that types `$1`, which over a table is `42P08`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
CREATE INDEX up_t_name_idx ON up_t (name);
-- @param $1 text x
-- @profile-lock
-- @test
-- @expect-error: the index is refused as a relation ahead of the parameter: 42809
SELECT 1 FROM up_t_name_idx WHERE $1 IS NULL AND $1 = 'x'::text;
