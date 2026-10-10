-- GROUP BY over an interval column folds comparison-estimate collisions: `30 days`
-- and `1 mon` are ONE group (Postgres's interval btree estimate), as are `1 hour`
-- and `60 minutes`. Regression guard that the interval grouping key uses estimate().
-- @setup
CREATE TABLE iv (id INT, v INTERVAL);
INSERT INTO iv VALUES
  (1, '30 days'), (2, '1 mon'),
  (3, '1 hour'), (4, '60 minutes'),
  (5, '2 hours');
-- @test
SELECT count(*) AS n FROM iv GROUP BY v ORDER BY n;
