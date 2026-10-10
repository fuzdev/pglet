-- NO ACTION accepts a substitute row: row 1 vacates key 1 and row 2 refills it, so no
-- child is ever left dangling and the swap succeeds (contrast RESTRICT, which rejects).
-- @profile-lock
-- @setup
CREATE TABLE onasub_p (id INT PRIMARY KEY);
CREATE TABLE onasub_c (id INT PRIMARY KEY, pid INT REFERENCES onasub_p(id));
INSERT INTO onasub_p VALUES (1), (2);
INSERT INTO onasub_c VALUES (10, 1);
UPDATE onasub_p SET id = CASE id WHEN 1 THEN 3 WHEN 2 THEN 1 END;

-- @test
-- The child still resolves to a live parent keyed 1.
SELECT
  (SELECT pid FROM onasub_c WHERE id = 10) AS child_pid,
  (SELECT count(*) FROM onasub_p WHERE id = 1) AS parent1;
