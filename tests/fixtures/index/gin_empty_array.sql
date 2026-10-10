-- `refs @> ARRAY[]::text[]` — the empty set is contained by every non-NULL array,
-- but `NULL @> '{}'` is NULL (excluded). pglet can't accelerate an empty
-- decomposition, so the GinScan returns every row and the recheck Filter applies
-- the real `@>` — matching Postgres (rows 1,2; the NULL-refs row 3 is dropped).
-- @profile-lock
-- @setup
CREATE TABLE gin_empty (id int4, refs text[]);
CREATE INDEX gin_empty_idx ON gin_empty USING gin(refs);
INSERT INTO gin_empty (id, refs) VALUES (1, '{a}'), (2, '{}'), (3, NULL);
-- @test
SELECT id FROM gin_empty WHERE refs @> ARRAY[]::text[] ORDER BY id;
