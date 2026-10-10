-- substr / substring over a bytea, beside a string literal in a COALESCE, a CASE, an
-- ARRAY and greatest: the literal takes the call's bytea type, value and declaration.
-- @setup
CREATE TABLE sbf (id INT, b BYTEA);
INSERT INTO sbf VALUES (1, NULL), (2, '\x01');
-- @test
SELECT id,
  coalesce(substr(b, 1), 'zz') AS c,
  CASE WHEN b IS NULL THEN 'zz' ELSE substring(b FROM 1) END AS k,
  ARRAY['zz', substr(b, 1)] AS a,
  greatest(substr(b, 1), 'zz') AS g,
  coalesce(substr(b, 1, 1), substr('\x02'::bytea, 1)) AS cc
FROM sbf ORDER BY id;
