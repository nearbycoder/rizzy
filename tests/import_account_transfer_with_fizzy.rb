# Run with Fizzy's Rails runner using an isolated copy of its primary SQLite
# database and a temporary Active Storage root. Do not point these paths at
# the running application's files.
database = File.expand_path(ENV.fetch("FIZZY_IMPORT_PROBE_DB"))
storage = File.expand_path(ENV.fetch("FIZZY_IMPORT_PROBE_STORAGE"))
archive = File.expand_path(ENV.fetch("RIZZY_ACCOUNT_ARCHIVE"))
live_database = Rails.root.join("storage/production.sqlite3").to_s
raise "Refusing the live Fizzy database" if database == live_database
raise "Refusing the live Fizzy storage" if storage.start_with?(Rails.root.join("storage").to_s)

ActiveRecord::Base.establish_connection(adapter: "sqlite3", database: database)
ActiveJob::Base.queue_adapter = :test
ActiveStorage::Blob.service.root = storage

account = Account.create!(name: "Rizzy transfer import probe")
File.open(archive, "rb") do |file|
  zip = ZipFile::Reader.new(file)
  Account::DataTransfer::Manifest.new(account).each_record_set do |record_set|
    record_set.import(from: zip)
  end
end

raise "No imported cards" unless account.cards.exists?
card = account.cards.find_by!(title: "Transfer card")
raise "Tagging missing" unless Tagging.where(account_id: account.id, card_id: card.id).exists?
raise "Assignment missing" unless Assignment.where(account_id: account.id, card_id: card.id).exists?
raise "Closure missing" unless Closure.where(account_id: account.id, card_id: card.id).exists?
raise "Goldness missing" unless Card::Goldness.where(account_id: account.id, card_id: card.id).exists?
raise "Watch missing" unless Watch.where(account_id: account.id, card_id: card.id).exists?
raise "Reaction missing" unless Reaction.where(account_id: account.id, reactable_id: card.id).exists?
raise "Board publication missing" unless Board::Publication.where(account_id: account.id).exists?
raise "Webhook missing" unless Webhook.where(account_id: account.id).exists?
raise "No imported storage blobs" unless ActiveStorage::Blob.where(account_id: account.id).exists?
ActiveStorage::Blob.where(account_id: account.id).find_each do |blob|
  raise "Missing imported file #{blob.id}" if blob.download.empty?
end
ActionText::RichText.where(account_id: account.id).find_each do |rich_text|
  raise "Unresolved portable blob reference" if rich_text.body.to_s.include?("gid://fizzy/ActiveStorage::Blob/")
end

puts "Fizzy imported #{account.cards.count} cards, #{Comment.where(account_id: account.id).count} comments, and #{ActiveStorage::Blob.where(account_id: account.id).count} blobs"
