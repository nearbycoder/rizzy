require "json"

# Run from the pinned source with: bin/rails runner /absolute/path/to/this/file
cases = [
  [ "America/New_York", "2026-03-08 12:00", "today" ],
  [ "America/New_York", "2026-11-01 12:00", "today" ],
  [ "America/Santiago", "2026-09-06 12:00", "today" ],
  [ "Pacific/Apia", "2011-12-29 12:00", "today" ],
  [ "Pacific/Apia", "2011-12-29 12:00", "this month" ],
  [ "America/Havana", "2026-03-08 12:00", "today" ],
  [ "America/Havana", "2026-11-01 12:00", "today" ]
]

cases.each do |zone, stamp, name|
  Time.use_zone(zone) do
    window = TimeWindowParser.new(now: Time.zone.parse(stamp)).parse(name)
    puts JSON.generate(
      zone: zone,
      name: name,
      start: window.begin.utc.iso8601(3),
      exclusive_end: (window.end + Rational(1, 1_000_000)).utc.iso8601(3)
    )
  end
end
