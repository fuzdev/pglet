-- A `numeric(p,s)[]` column rounds each element to the declared scale on store. The
-- column used to resolve to a scalar `numeric`, which made this literal `22P02` and
-- made a bare `1.25` store unrounded.
-- @setup
CREATE TABLE tmnu (a numeric(4,1)[]);
INSERT INTO tmnu VALUES ('{1.25,2.5}');
-- @test
SELECT a FROM tmnu;
