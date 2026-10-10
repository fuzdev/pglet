-- An element's bytes cannot stand in for the key's own structure: a `text[]` element
-- holding the byte `0x01` or `0x02` is one element, not an element boundary and not a
-- NULL element, so `{a,b}` beside the one-element array `a\x01b`, and `{a,NULL}` beside
-- `a\x02`, are four distinct keys.
-- @profile-lock
-- @setup
CREATE TABLE ckb_t (id INT, tags TEXT[] UNIQUE);
INSERT INTO ckb_t (tags, id) VALUES ('{a,b}', 1), (ARRAY[E'a\x01b'], 2), ('{a,NULL}', 3), (ARRAY[E'a\x02'], 4);
-- @test
SELECT id, length(tags::text) AS n FROM ckb_t ORDER BY id;
