-- @test
-- @text
-- The **value** half of `types/unnest_timestamptz_declared_type.sql`. pglet's text
-- encoder is column-type-aware (`value_to_text_with_oid`), so a `timestamptz` column an
-- unnest declared as plain `timestamp` renders **without** the UTC `+00` offset — and a
-- text-protocol driver then reparses the value in its own local zone and silently shifts
-- the instant. That is the class `types/timestamptz_text.sql` guards for a base-table
-- column; this reaches it through a FROM-clause `unnest(…)` instead, where the column
-- type comes from the array expression rather than from the catalog.
--
-- Only the text (simple-query) leg can see it: the binary encoder writes the same eight
-- bytes either way and the extended protocol's `Describe` already declared 1184, so the
-- offset is the whole observable. The harness pins both sessions to `TimeZone=UTC`, so
-- pglet (always UTC) matches Postgres byte-for-byte. Covers fractional seconds and whole
-- seconds.
SELECT t FROM unnest(ARRAY['2024-06-15T12:30:45.123Z'::timestamptz,
                           '2024-01-02T03:04:05Z'::timestamptz]) AS u(t);
