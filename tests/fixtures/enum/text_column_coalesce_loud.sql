-- `COALESCE` folds its branches through 18.6's `select_common_type`, which refuses a pair
-- whose **categories** differ: an enum is `E` where `text` is `S`. Reading the OID alone
-- made both `text`, so the fold succeeded and the node was handed the enum's order.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for mood and text
SELECT COALESCE(m, s) FROM t ORDER BY id;
