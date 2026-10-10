-- The spine's bound containment path with a GIN index: `refs @> $1` over gin(refs),
-- the parameter bound over the extended protocol. (Rows inserted AFTER create index
-- → per-write maintenance path.)
-- @profile-lock
-- @setup
CREATE TABLE gin_refs_p (id int4, refs text[]);
CREATE INDEX gin_refs_p_idx ON gin_refs_p USING gin(refs);
INSERT INTO gin_refs_p (id, refs) VALUES (1, '{a,b}'), (2, '{c}'), (3, '{a,c}');
-- @param $1 text[] {a}
-- @test
SELECT id FROM gin_refs_p WHERE refs @> $1 ORDER BY id;
