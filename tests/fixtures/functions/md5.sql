-- md5 over text (its UTF-8 bytes) and bytea (its own, NULs included): the empty string,
-- a multibyte string, the padding boundaries at 55, 56, 63, 64 and 65 bytes, and a
-- 100-byte input spanning two blocks; NULL is NULL. Lower-case hex text.
-- @setup
CREATE TABLE m5 (id INT, t TEXT, b BYTEA);
INSERT INTO m5 VALUES
  (1, '', '\x'),
  (2, 'a', '\x00610062'),
  (3, 'héllo wörld', '\x00'),
  (4, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', NULL),
  (5, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', NULL),
  (6, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', NULL),
  (7, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', NULL),
  (8, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', NULL),
  (9, 'The quick brown fox jumps over the lazy dog, then over the lazy dog once more, slowly.', NULL),
  (10, NULL, NULL);
-- @test
SELECT id, octet_length(t) AS bytes, md5(t), md5(b) AS mb FROM m5 ORDER BY id;
