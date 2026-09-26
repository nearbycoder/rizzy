# Run only with DATABASE_URL pointing at a disposable copy of the pinned source DB.
raise "disposable source database required" unless ENV["RIZZY_DISPOSABLE_SOURCE_DB"] == "1"

user = User.where(role: :owner, active: true).find do |candidate|
  candidate.identity && candidate.boards.exists?
end
raise "active owner with board required" unless user

session = ActionDispatch::Integration::Session.new(Rails.application)
session.host! "localhost"
session.post "/session", params: { email_address: user.identity.email_address }, as: :json
raise "session request failed" unless session.response.created?
pending = session.response.parsed_body.fetch("pending_authentication_token")
code = MagicLink.order(created_at: :desc).first.code
session.post "/session/magic_link", params: { code: code, pending_authentication_token: pending }, as: :json
raise "magic link failed" unless session.response.successful?

board = user.boards.first
path = "/#{user.account.external_account_id}/boards/#{board.id}"
etags = {}
[nil, "America/New_York", "Eastern Time (US & Canada)", "Madrid", "Europe/Madrid", "Not/A_Time_Zone"].each do |name|
  session.cookies["timezone"] = name
  session.get path, headers: { "Accept" => "text/html" }
  raise "board request failed for #{name.inspect}: #{session.response.status}" unless session.response.successful?
  etags[name] = session.response.headers.fetch("ETag")
end
raise "invalid timezone changed validator" unless etags[nil] == etags["Not/A_Time_Zone"]
raise "alias failed to change validator" if etags["Eastern Time (US & Canada)"] == etags[nil]
raise "alias should have a distinct validator" if etags["Eastern Time (US & Canada)"] == etags["America/New_York"]
raise "Madrid alias should have a distinct validator" if etags["Madrid"] == etags["Europe/Madrid"]
puts "Pinned Fizzy timezone aliases have distinct ETags from their IANA zones; invalid input preserves the original ETag"
