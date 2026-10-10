-- `jsonb_build_object(k1, v1, …)` builds a JSONB object from alternating
-- key/value arguments; a text value becomes a JSON *string* (quoted). Asserted
-- via containment (`@>`) and extraction (`->>`) predicates, which are stable
-- across jsonb text-format normalization (no raw-object projection).
-- @test
SELECT
  -- multi-key build, read back a field
  jsonb_build_object('kind', 'doc', 'n', 'x') ->> 'kind'     AS kind,
  -- a NULL value member is kept; `->>` of a JSON null yields SQL NULL
  jsonb_build_object('k', NULL) ->> 'k'                      AS null_member,
  -- @> true: single-key subset
  jsonb_build_object('kind', 'doc') @> jsonb_build_object('kind', 'doc') AS contained,
  -- @> true: multi-key container contains single-key subset
  jsonb_build_object('kind', 'doc', 'n', 'x') @> jsonb_build_object('kind', 'doc') AS contained_multi,
  -- @> false: value mismatch
  jsonb_build_object('a', '1') @> jsonb_build_object('a', '2') AS not_contained,
  -- @> false: key absent from container
  jsonb_build_object('kind', 'doc') @> jsonb_build_object('other', 'x') AS missing_key;
