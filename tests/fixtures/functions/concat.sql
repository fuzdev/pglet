-- concat / concat_ws skip NULL arguments (contributing nothing, not even a
-- separator gap); non-text arguments are stringified. concat_ws's separator is
-- strict — a NULL separator makes the whole call NULL.
-- @setup
CREATE TABLE c (id INT, a TEXT, b TEXT, n INT);
INSERT INTO c VALUES (1, 'a', 'b', 2), (2, 'a', NULL, 22), (3, NULL, NULL, NULL);
-- @test
SELECT id,
  concat(a, n, b) AS cc,
  concat(a, NULL, b) AS cn,
  concat_ws(',', a, n, b) AS cw,
  concat_ws('-', a, NULL, b) AS cwn,
  concat_ws(NULL, a, b) AS cwnull
FROM c ORDER BY id;
