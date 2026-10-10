-- A non-aggregated column that is neither grouped nor functionally dependent on
-- the GROUP BY fails loud (42803), matching Postgres, instead of silently
-- returning an arbitrary group row.
-- @expect-error: `other` must appear in GROUP BY or an aggregate (grouping_error)
-- @profile-lock
-- @setup
CREATE TABLE grp_bad (pk INT PRIMARY KEY, name TEXT, other TEXT);
INSERT INTO grp_bad VALUES (1, 'a', 'x'), (2, 'a', 'y');
-- @test
SELECT other, COUNT(*) FROM grp_bad GROUP BY name;
