-- @setup
CREATE TABLE people (id INT, name TEXT, nick TEXT);
INSERT INTO people VALUES (1, 'Ada', 'A'), (2, 'Bo', NULL);

-- @test
-- `||` renders each operand to its text form and concatenates; a NULL operand
-- propagates to NULL; chains left-associatively; the spine's prefix-search shape
-- (`name LIKE prefix || '%'`) binds `||` tighter than LIKE.
SELECT id,
       name || '!' AS bang,
       name || ' ' || nick AS joined,
       'id=' || id AS labeled
FROM people
WHERE name LIKE 'A' || '%'
ORDER BY id ASC;
