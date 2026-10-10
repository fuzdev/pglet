-- ON UPDATE RESTRICT never accepts a substitute row: row 1 vacates key 1 while row 2
-- refills it, but RESTRICT still rejects (23001) because a child referenced 1.
-- @profile-lock
-- @setup
CREATE TABLE oursub_p (id INT PRIMARY KEY);
CREATE TABLE oursub_c (id INT PRIMARY KEY, pid INT REFERENCES oursub_p(id) ON UPDATE RESTRICT);
INSERT INTO oursub_p VALUES (1), (2);
INSERT INTO oursub_c VALUES (10, 1);

-- @test
-- @expect-error: RESTRICT never substitutes, so the swap is rejected (23001)
UPDATE oursub_p SET id = CASE id WHEN 1 THEN 3 WHEN 2 THEN 1 END;
