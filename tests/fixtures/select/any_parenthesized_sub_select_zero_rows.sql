-- pglet refuses the `ANY ((SELECT …))` sub-select form where it is evaluated, so over no
-- rows it is never evaluated and the statement answers as on 18.6.
-- @setup
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1);
CREATE TABLE e (a INT);
-- @test
SELECT a = ANY ((SELECT a FROM r)) AS m FROM e;
