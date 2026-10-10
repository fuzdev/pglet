-- ...and it moves only the counter. `seqstart` keeps the declared START WITH, so a
-- later bare RESTART returns there — the fact that separates "honour RESTART" from
-- "fold it into the start".
-- @setup
CREATE SEQUENCE crst2 START WITH 10 RESTART WITH 5;
SELECT nextval('crst2');
ALTER SEQUENCE crst2 RESTART;
-- @test
SELECT nextval('crst2');
