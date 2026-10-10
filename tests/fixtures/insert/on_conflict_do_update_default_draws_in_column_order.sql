-- Two `DEFAULT`s in a `DO UPDATE SET` list draw on one sequence in column order: `a`
-- takes 3 and `b` 4. pglet answered `4 | 3`.
-- @setup
CREATE SEQUENCE ocdo_seq;
CREATE TABLE ocdo (id INT PRIMARY KEY, a INT DEFAULT nextval('ocdo_seq'), b INT DEFAULT nextval('ocdo_seq'));
INSERT INTO ocdo (id) VALUES (1);

-- @test
INSERT INTO ocdo VALUES (1, 0, 0)
ON CONFLICT (id) DO UPDATE SET b = DEFAULT, a = DEFAULT
RETURNING id, a, b;
