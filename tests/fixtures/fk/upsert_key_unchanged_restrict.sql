-- An `ON CONFLICT DO UPDATE` that assigns a referenced column its own value changes no key,
-- so an `ON UPDATE RESTRICT` child does not refuse it.
-- @profile-lock
-- @setup
CREATE TABLE upku_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upku_c (pid INT REFERENCES upku_p(id) ON UPDATE RESTRICT, x INT);
INSERT INTO upku_p VALUES (1, 1), (2, 2);
INSERT INTO upku_c VALUES (1, 10);

-- @test
INSERT INTO upku_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = upku_p.id, v = 7 RETURNING id, v;
