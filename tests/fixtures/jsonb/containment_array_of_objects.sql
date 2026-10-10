-- A **container** element of the right operand is matched against the left's
-- container elements, recursively — so an array of objects contains an array
-- holding a sub-object of any one of them.
--
-- Two rules this pins, both from `JsonbDeepContains`:
--   * a container rhs element never matches a *scalar* lhs element (Postgres builds
--     a temp array of only the lhs containers and gives up when it is empty), so
--     `'[1,2,3]' @> '[[1]]'` is false;
--   * the match is per-element, so `[{"a":1,"b":2}] @> [{"b":2}]` holds — the rhs
--     object need only be contained by *some* lhs element.
-- @setup
CREATE TABLE jcao (id INT, d JSONB);
INSERT INTO jcao VALUES
  (1, '[{"a":1},{"b":2}]'),
  (2, '[{"a":9}]'),
  (3, '[1,2,3]');
-- @test
SELECT
  ('[{"a":1},{"b":2}]'::jsonb @> '[{"a":1}]'::jsonb) AS object_element,
  ('[{"a":1,"b":2}]'::jsonb @> '[{"b":2}]'::jsonb) AS partial_object_element,
  ('[{"a":1},{"b":2}]'::jsonb @> '[{"a":1},{"b":2}]'::jsonb) AS both_elements,
  ('[{"a":1}]'::jsonb @> '[{"a":1,"b":2}]'::jsonb) AS element_not_contained,
  ('[1,2,3]'::jsonb @> '[[1]]'::jsonb) AS no_container_element,
  (SELECT count(*) FROM jcao WHERE d @> '[{"a":1}]'::jsonb) AS from_column;
