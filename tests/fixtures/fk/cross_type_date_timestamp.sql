-- A child DATE column REFERENCES a parent TIMESTAMP PK. Postgres RI casts the date to
-- timestamp (midnight UTC), so a date whose midnight matches a parent timestamp is
-- accepted — the covering-index probe coerces date -> timestamp before the lookup.
-- @setup
CREATE TABLE cdt_parent (ts TIMESTAMP PRIMARY KEY);
CREATE TABLE cdt_child (d DATE REFERENCES cdt_parent(ts));
INSERT INTO cdt_parent VALUES ('2024-01-01 00:00:00');

-- @test
INSERT INTO cdt_child VALUES ('2024-01-01');
SELECT d FROM cdt_child;
