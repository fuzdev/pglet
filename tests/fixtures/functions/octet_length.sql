-- octet_length counts bytes: a multibyte character counts its UTF-8 length; a bytea
-- counts its NULs; int4; NULL is NULL. The unaliased column is `octet_length`.
-- @setup
CREATE TABLE ol (id INT, t TEXT, v VARCHAR(20), b BYTEA);
INSERT INTO ol VALUES
  (1, 'abc', 'abc', '\x616263'),
  (2, 'héllo', 'wörld', '\x000000'),
  (3, '', '', '\x'),
  (4, NULL, NULL, NULL);
-- @test
SELECT id, octet_length(t), octet_length(v) AS ov, octet_length(b) AS ob,
  octet_length('日本') AS lit
FROM ol ORDER BY id;
