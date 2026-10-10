-- The same rule at each of the five branch constructors: a wildcard subquery over an
-- `integer` relation beside a `numeric` constant declares `numeric` (`numeric[]` for the
-- array), as 18.6 does, rather than declaring the subquery's own `integer`.
-- @setup
CREATE TABLE sswi (c INT);
INSERT INTO sswi VALUES (7);
-- @test
SELECT COALESCE((SELECT * FROM sswi), 2.5) AS a,
       CASE WHEN true THEN (SELECT * FROM sswi) ELSE 2.5 END AS b,
       ARRAY[(SELECT * FROM sswi), 2.5] AS c,
       greatest((SELECT * FROM sswi), 2.5) AS d,
       least((SELECT * FROM sswi), 2.5) AS e,
       nullif((SELECT * FROM sswi), 2.5) AS f;
