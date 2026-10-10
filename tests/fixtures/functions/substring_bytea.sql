-- substring over bytea — every spelling (FROM … FOR, FROM, FOR, the comma form) — in
-- bytes, answering bytea; a column and a literal.
-- @setup
CREATE TABLE sgb (id INT, b BYTEA);
INSERT INTO sgb VALUES (1, '\x616263'), (2, '\x00010203ff'), (3, NULL);
-- @test
SELECT id,
  substring(b FROM 2 FOR 1) AS f1,
  substring(b FROM 2) AS f2,
  substring(b FOR 2) AS f3,
  substring(b, 2, 2) AS c1,
  substring(b, -1, 3) AS clip,
  substring('\x616263'::bytea, 3) AS lit
FROM sgb ORDER BY id;
