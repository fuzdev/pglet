-- split_part: 1-based; a negative n counts from the end (-1 = last); an
-- out-of-range n yields ''; an empty delimiter treats the whole string as field
-- 1/-1; the delimiter is a literal full-string match.
-- @setup
CREATE TABLE sp (id INT, t TEXT);
INSERT INTO sp VALUES (1, 'a,b,c'), (2, 'a~@~b~@~c'), (3, NULL);
-- @test
SELECT id,
  split_part(t, ',', 2) AS p2,
  split_part(t, ',', -1) AS plast,
  split_part(t, ',', 9) AS poor,
  split_part(t, '', 1) AS pempty,
  split_part('a~@~b~@~c', '~@~', 2) AS multi
FROM sp ORDER BY id;
