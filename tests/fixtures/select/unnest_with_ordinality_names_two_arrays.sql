-- Over two arrays an alias list shorter than the columns names the first ones and leaves
-- the rest theirs: the second array's `unnest`, then `ordinality`. The arrays zip to the
-- longest, NULL-padded, and the ordinality numbers every row.
-- @test
SELECT * FROM unnest(ARRAY[1, 2], ARRAY['a', 'b', 'c']) WITH ORDINALITY AS z(x) ORDER BY 3;
