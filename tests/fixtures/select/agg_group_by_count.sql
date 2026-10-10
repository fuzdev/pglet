-- @setup
CREATE TABLE events (id INT, type TEXT, status TEXT);
INSERT INTO events VALUES (1, 'click', 'ok'), (2, 'view', 'ok'), (3, 'click', 'ok'), (4, 'click', 'err'), (5, 'view', 'ok');

-- @test
SELECT type, COUNT(*) FROM events GROUP BY type ORDER BY type;
