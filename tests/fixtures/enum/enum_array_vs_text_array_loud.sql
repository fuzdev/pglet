-- One type constructor up. `mood[]` is `typcategory` `A` like `text[]`, so the pair is a
-- same-category **conversion** failure rather than a category mismatch — `42846`, not
-- `42804`, and the array's identity is its element's.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: cannot convert text[] to mood[]
-- @expect-message
SELECT COALESCE(ARRAY[m], ARRAY[s]) FROM t ORDER BY id;
