# Run through Fizzy's Rails runner against a Rizzy account ZIP. This calls
# every source record set's read-only validator; it does not import records.
archive = ENV.fetch("RIZZY_ACCOUNT_ARCHIVE")
account = Account.find_by!(external_account_id: Integer(ENV.fetch("FIZZY_ACCOUNT_ID")))

File.open(archive, "rb") do |file|
  zip = ZipFile::Reader.new(file)
  Account::DataTransfer::Manifest.new(account).each_record_set do |record_set|
    record_set.check(from: zip)
  end
end

puts "Fizzy accepted the account ZIP structure"
