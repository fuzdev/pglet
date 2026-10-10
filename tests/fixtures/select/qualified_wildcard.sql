-- @setup
CREATE TABLE al (id INT, seq INT, msg TEXT);
CREATE TABLE acc (id INT, username TEXT);
INSERT INTO al VALUES (1, 10, 'x'), (2, 20, 'y');
INSERT INTO acc VALUES (1, 'ada'), (2, 'bo');

-- @test
-- `al.*` expands to only that relation's columns (its side of the JOIN), mixed
-- with a computed column — the spine's `SELECT al.*, <resolved> FROM audit_log al
-- LEFT JOIN …` shape.
SELECT al.*, acc.username AS resolved
FROM al
LEFT JOIN acc ON acc.id = al.id
ORDER BY al.seq DESC;
