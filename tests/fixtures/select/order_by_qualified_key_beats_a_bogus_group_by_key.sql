-- The **control** for the three error-order fixtures beside it: the one neighbour a sort
-- key still outranks. `transformSortClause` runs *before* `transformGroupClause`, so the
-- key's missing relation is reported and not the `GROUP BY` key's missing column — `42P01`
-- on both engines, and unchanged by moving the keys below the projection.
--
-- Without this, "resolve the keys after the projection" and "resolve them last" are
-- indistinguishable, and only one of them is Postgres's order.
-- @setup
CREATE TABLE sk_epg (a INT, s TEXT);
INSERT INTO sk_epg VALUES (1, 'x'), (2, 'y');

-- @expect-error: missing FROM-clause entry for table "bogusrel" (42P01)
-- @test
SELECT count(*) FROM sk_epg GROUP BY bogusg ORDER BY bogusrel.a;
