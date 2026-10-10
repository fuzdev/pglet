-- The accept side: an empty or NULL expansion beside a volatile sibling that cannot fail
-- emits no row.
-- @test
SELECT count(*) AS n FROM (SELECT unnest(ARRAY[]::int[]) AS u, random() AS r) s;
