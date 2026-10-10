-- A `bytea[]` key's elements are byte strings that can hold any byte: a NUL inside an
-- element is not the element's end, `0x01` is not an element boundary, and one element
-- can be a prefix of another. Six distinct keys, in `bytea`'s byte order element by
-- element — the empty string below `\x00`, which is below `\x0000`.
-- @profile-lock
-- @setup
CREATE TABLE cko_b (id INT, a BYTEA[] PRIMARY KEY);
INSERT INTO cko_b (a, id) VALUES
  (ARRAY['\x00'::bytea], 1), (ARRAY[''::bytea, '\x61'::bytea], 2), (ARRAY['\x0000'::bytea], 3),
  (ARRAY['\x00ff'::bytea], 4), (ARRAY['\x610162'::bytea], 5), (ARRAY['\x61'::bytea, '\x62'::bytea], 6),
  (ARRAY['\x01'::bytea], 7), (ARRAY[''::bytea], 8);
-- @test
SELECT id FROM cko_b ORDER BY a;
