-- @setup
CREATE TABLE scores (name TEXT, score INT);
INSERT INTO scores VALUES
  ('Alice', 100),
  ('Bob', 90),
  ('Carol', 90),
  ('Dave', 80),
  ('Eve', 80);

-- @test
SELECT name, score, RANK() OVER (ORDER BY score DESC) AS rank
FROM scores
ORDER BY rank, name;
