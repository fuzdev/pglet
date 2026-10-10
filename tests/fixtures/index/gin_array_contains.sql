-- GIN-accelerated array containment: `refs @> ARRAY[...]` over a non-partial
-- gin(refs) index. pglet plans a GinScan + recheck Filter; Postgres uses its GIN.
-- The result rows must match. (Index created AFTER the rows → back-fill path.)
-- @profile-lock
-- @setup
CREATE TABLE gin_refs (id int4, refs text[]);
INSERT INTO gin_refs (id, refs) VALUES (1, '{a,b}'), (2, '{c}'), (3, '{a,c}'), (4, '{}');
CREATE INDEX gin_refs_idx ON gin_refs USING gin(refs);
-- @test
SELECT id FROM gin_refs WHERE refs @> ARRAY['a','c'] ORDER BY id;
