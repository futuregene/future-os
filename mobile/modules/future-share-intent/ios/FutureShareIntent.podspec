Pod::Spec.new do |s|
  s.name = 'FutureShareIntent'
  s.version = '1.0.0'
  s.summary = 'FutureOS shared App Group inbox reader'
  s.description = s.summary
  s.license = { :type => 'MIT' }
  s.author = 'FutureOS'
  s.homepage = 'https://github.com/futuregene/future-os'
  s.source = { :git => 'https://github.com/futuregene/future-os.git' }
  s.platforms = { :ios => '16.4' }
  s.swift_version = '5.0'
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.source_files = '*.swift'
end
