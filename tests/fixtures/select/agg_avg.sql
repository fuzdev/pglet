-- @setup
CREATE TABLE scores (id INT, score FLOAT);
INSERT INTO scores VALUES (1, 80.0), (2, 90.0), (3, 100.0);

-- @test
SELECT AVG(score) FROM scores;
