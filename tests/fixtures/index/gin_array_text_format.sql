-- Same GIN array containment, but compared over the simple-query (text) protocol so
-- the text-format encoder is exercised — the array column itself is projected, so
-- both the GIN filtering and the `{a,b}` array text rendering must match Postgres.
-- @text
-- @profile-lock
-- @setup
CREATE TABLE gin_refs_txt (id int4, refs text[]);
INSERT INTO gin_refs_txt (id, refs) VALUES (1, '{a,b}'), (2, '{c}'), (3, '{a,c}');
CREATE INDEX gin_refs_txt_idx ON gin_refs_txt USING gin(refs);
-- @test
SELECT id, refs FROM gin_refs_txt WHERE refs @> ARRAY['a'] ORDER BY id;
