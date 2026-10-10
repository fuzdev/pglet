-- `sum(interval)` / `avg(interval)` are real Postgres aggregates that pglet used to
-- swallow into the f64 accumulator and return NULL from. `sum` is field-wise
-- `interval_pl` (no justification pass, so `1 mon` + `1 day` stays `1 mon 1 day`);
-- `avg` divides that sum by the non-NULL count with `interval_div`'s scaling.
-- @setup
CREATE TABLE spans (d INTERVAL);
INSERT INTO spans VALUES ('1 mon'), ('1 day'), ('2 hours'), (NULL);
-- @test
SELECT SUM(d) AS total, AVG(d) AS mean, MIN(d) AS lo, MAX(d) AS hi FROM spans;
