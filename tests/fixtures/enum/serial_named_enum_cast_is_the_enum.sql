-- `serial` is no type on 18.6 — only a column definition's rewrite — so an enum named
-- `serial` is what the cast names: a builtin's name yields to the builtin, and this one is
-- not a builtin's.
-- @setup
CREATE TYPE serial AS ENUM ('s');

-- @test
SELECT ('s'::serial)::text AS a;
