use chrono::{Local, NaiveDateTime, TimeZone, Utc};

fn timezone_abbr() -> String {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|tz| {
            let abbr = match tz.as_str() {
                // UTC
                "UTC" | "Etc/UTC" | "Etc/GMT" => "UTC",

                // Asia
                "Asia/Seoul" => "KST",
                "Asia/Tokyo" => "JST",
                "Asia/Shanghai" | "Asia/Chongqing" | "Asia/Harbin" | "Asia/Urumqi"
                | "Asia/Chungking" => "CST",
                "Asia/Hong_Kong" => "HKT",
                "Asia/Taipei" => "CST",
                "Asia/Macau" | "Asia/Macao" => "CST",
                "Asia/Kolkata" | "Asia/Calcutta" => "IST",
                "Asia/Colombo" => "IST",
                "Asia/Kathmandu" | "Asia/Katmandu" => "NPT",
                "Asia/Dhaka" | "Asia/Dacca" => "BST",
                "Asia/Yangon" | "Asia/Rangoon" => "MMT",
                "Asia/Bangkok" | "Asia/Phnom_Penh" | "Asia/Vientiane" => "ICT",
                "Asia/Ho_Chi_Minh" | "Asia/Saigon" => "ICT",
                "Asia/Jakarta" | "Asia/Pontianak" => "WIB",
                "Asia/Makassar" | "Asia/Ujung_Pandang" => "WITA",
                "Asia/Jayapura" => "WIT",
                "Asia/Singapore" => "SGT",
                "Asia/Kuala_Lumpur" | "Asia/Kuching" => "MYT",
                "Asia/Manila" => "PHT",
                "Asia/Brunei" => "BNT",
                "Asia/Karachi" => "PKT",
                "Asia/Tashkent" | "Asia/Samarkand" => "UZT",
                "Asia/Almaty" | "Asia/Qostanay" => "ALMT",
                "Asia/Yekaterinburg" => "YEKT",
                "Asia/Omsk" => "OMST",
                "Asia/Novosibirsk" => "NOVT",
                "Asia/Krasnoyarsk" => "KRAT",
                "Asia/Irkutsk" => "IRKT",
                "Asia/Yakutsk" | "Asia/Chita" => "YAKT",
                "Asia/Vladivostok" => "VLAT",
                "Asia/Magadan" | "Asia/Sakhalin" => "MAGT",
                "Asia/Kamchatka" | "Asia/Anadyr" => "PETT",
                "Asia/Dubai" | "Asia/Muscat" => "GST",
                "Asia/Riyadh" | "Asia/Aden" | "Asia/Kuwait" | "Asia/Qatar"
                | "Asia/Bahrain" => "AST",
                "Asia/Tehran" => "IRST",
                "Asia/Kabul" => "AFT",
                "Asia/Baku" => "AZT",
                "Asia/Tbilisi" => "GET",
                "Asia/Yerevan" => "AMT",
                "Asia/Jerusalem" | "Asia/Tel_Aviv" => "IST",
                "Asia/Beirut" => "EET",
                "Asia/Damascus" => "EET",
                "Asia/Amman" => "EET",
                "Asia/Baghdad" => "AST",
                "Asia/Ulaanbaatar" | "Asia/Ulan_Bator" => "ULAT",
                "Asia/Hovd" => "HOVT",
                "Asia/Choibalsan" => "CHOT",
                "Asia/Thimphu" | "Asia/Thimbu" => "BTT",
                "Asia/Dili" => "TLT",
                "Asia/Bishkek" => "KGT",
                "Asia/Dushanbe" => "TJT",
                "Asia/Ashgabat" | "Asia/Ashkhabad" => "TMT",

                // Americas
                "America/New_York" | "US/Eastern" | "America/Detroit"
                | "America/Kentucky/Louisville" | "America/Kentucky/Monticello"
                | "America/Indiana/Indianapolis" | "America/Indiana/Vincennes"
                | "America/Indiana/Winamac" | "America/Indiana/Marengo"
                | "America/Indiana/Petersburg" | "America/Indiana/Vevay" => "EST",
                "America/Chicago" | "US/Central" | "America/Indiana/Knox"
                | "America/Indiana/Tell_City" | "America/Menominee"
                | "America/North_Dakota/Beulah" | "America/North_Dakota/Center"
                | "America/North_Dakota/New_Salem" => "CST",
                "America/Denver" | "US/Mountain" | "America/Boise" => "MST",
                "America/Phoenix" | "US/Arizona" => "MST",
                "America/Los_Angeles" | "US/Pacific" => "PST",
                "America/Anchorage" | "US/Alaska" | "America/Juneau" | "America/Sitka"
                | "America/Yakutat" | "America/Nome" | "America/Metlakatla" => "AKST",
                "America/Adak" | "US/Aleutian" => "HST",
                "Pacific/Honolulu" | "US/Hawaii" => "HST",
                "America/Toronto" | "America/Montreal" | "America/Nipigon"
                | "America/Thunder_Bay" | "America/Iqaluit" | "America/Pangnirtung" => "EST",
                "America/Winnipeg" | "America/Rainy_River"
                | "America/Resolute" | "America/Rankin_Inlet" => "CST",
                "America/Edmonton" | "America/Cambridge_Bay"
                | "America/Yellowknife" | "America/Inuvik" => "MST",
                "America/Vancouver" | "America/Dawson_Creek"
                | "America/Fort_Nelson" | "America/Whitehorse"
                | "America/Dawson" | "America/Creston" => "PST",
                "America/St_Johns" => "NST",
                "America/Halifax" | "America/Glace_Bay" | "America/Moncton"
                | "America/Goose_Bay" | "Atlantic/Bermuda" => "AST",
                "America/Regina" | "America/Swift_Current" => "CST",
                "America/Mexico_City" | "America/Merida" | "America/Monterrey"
                | "America/Bahia_Banderas" => "CST",
                "America/Cancun" => "EST",
                "America/Tijuana" => "PST",
                "America/Hermosillo" => "MST",
                "America/Chihuahua" | "America/Mazatlan" | "America/Ojinaga" => "MST",
                "America/Guatemala" | "America/Belize" | "America/Costa_Rica"
                | "America/El_Salvador" | "America/Tegucigalpa"
                | "America/Managua" => "CST",
                "America/Panama" | "America/Bogota" | "America/Lima"
                | "America/Guayaquil" | "America/Jamaica"
                | "America/Cayman" => "EST",
                "America/Havana" => "CST",
                "America/Caracas" => "VET",
                "America/La_Paz" => "BOT",
                "America/Santiago" => "CLT",
                "America/Asuncion" => "PYT",
                "America/Montevideo" => "UYT",
                "America/Argentina/Buenos_Aires" | "America/Argentina/Cordoba"
                | "America/Argentina/Salta" | "America/Argentina/Jujuy"
                | "America/Argentina/Tucuman" | "America/Argentina/Catamarca"
                | "America/Argentina/La_Rioja" | "America/Argentina/San_Juan"
                | "America/Argentina/Mendoza" | "America/Argentina/San_Luis"
                | "America/Argentina/Rio_Gallegos"
                | "America/Argentina/Ushuaia" => "ART",
                "America/Sao_Paulo" | "America/Recife" | "America/Fortaleza"
                | "America/Bahia" | "America/Belem" | "America/Maceio"
                | "America/Araguaina" => "BRT",
                "America/Manaus" | "America/Porto_Velho"
                | "America/Boa_Vista" | "America/Campo_Grande"
                | "America/Cuiaba" => "AMT",
                "America/Noronha" => "FNT",
                "America/Rio_Branco" | "America/Eirunepe" => "ACT",
                "America/Guyana" => "GYT",
                "America/Paramaribo" => "SRT",
                "America/Cayenne" => "GFT",
                "America/Port_of_Spain" | "America/Martinique"
                | "America/Guadeloupe" | "America/Barbados"
                | "America/Curacao" | "America/Aruba"
                | "America/Puerto_Rico" | "America/Virgin"
                | "America/Dominica" | "America/Grenada"
                | "America/St_Kitts" | "America/St_Lucia"
                | "America/St_Vincent" | "America/Antigua"
                | "America/Anguilla" | "America/Montserrat"
                | "America/Tortola" | "America/St_Thomas" => "AST",
                "America/Santo_Domingo" => "AST",
                "America/Port-au-Prince" => "EST",

                // Europe
                "Europe/London" | "Europe/Belfast" | "Europe/Guernsey"
                | "Europe/Isle_of_Man" | "Europe/Jersey" => "GMT",
                "Europe/Dublin" => "GMT",
                "Europe/Lisbon" | "Atlantic/Madeira" => "WET",
                "Atlantic/Canary" | "Atlantic/Faroe" => "WET",
                "Europe/Paris" | "Europe/Berlin" | "Europe/Amsterdam"
                | "Europe/Brussels" | "Europe/Luxembourg"
                | "Europe/Zurich" | "Europe/Vienna" | "Europe/Rome"
                | "Europe/Madrid" | "Europe/Monaco" | "Europe/Andorra"
                | "Europe/Belgrade" | "Europe/Bratislava" | "Europe/Budapest"
                | "Europe/Copenhagen" | "Europe/Gibraltar" | "Europe/Ljubljana"
                | "Europe/Malta" | "Europe/Oslo" | "Europe/Prague"
                | "Europe/San_Marino" | "Europe/Sarajevo" | "Europe/Skopje"
                | "Europe/Stockholm" | "Europe/Tirane" | "Europe/Vaduz"
                | "Europe/Vatican" | "Europe/Warsaw" | "Europe/Zagreb"
                | "Europe/Podgorica" => "CET",
                "Europe/Athens" | "Europe/Bucharest" | "Europe/Helsinki"
                | "Europe/Kiev" | "Europe/Kyiv" | "Europe/Riga"
                | "Europe/Sofia" | "Europe/Tallinn" | "Europe/Vilnius"
                | "Europe/Chisinau" | "Europe/Mariehamn"
                | "Europe/Uzhgorod" | "Europe/Zaporozhye" => "EET",
                "Europe/Istanbul" => "TRT",
                "Europe/Moscow" | "Europe/Kirov" | "Europe/Simferopol" => "MSK",
                "Europe/Volgograd" => "MSK",
                "Europe/Samara" | "Europe/Ulyanovsk" => "SAMT",
                "Europe/Kaliningrad" => "EET",
                "Europe/Minsk" => "MSK",
                "Europe/Saratov" | "Europe/Astrakhan" => "MSK+1",
                "Atlantic/Reykjavik" => "GMT",
                "Atlantic/Azores" => "AZOT",
                "Atlantic/Cape_Verde" => "CVT",
                "Atlantic/South_Georgia" => "GST",

                // Africa
                "Africa/Cairo" => "EET",
                "Africa/Johannesburg" | "Africa/Harare"
                | "Africa/Maputo" | "Africa/Lusaka"
                | "Africa/Blantyre" | "Africa/Bujumbura"
                | "Africa/Gaborone" | "Africa/Kigali"
                | "Africa/Lubumbashi" | "Africa/Windhoek" => "SAST",
                "Africa/Lagos" | "Africa/Bangui" | "Africa/Brazzaville"
                | "Africa/Douala" | "Africa/Kinshasa" | "Africa/Libreville"
                | "Africa/Luanda" | "Africa/Malabo" | "Africa/Niamey"
                | "Africa/Ndjamena" | "Africa/Porto-Novo"
                | "Africa/Tunis" | "Africa/Algiers" => "WAT",
                "Africa/Nairobi" | "Africa/Addis_Ababa"
                | "Africa/Asmara" | "Africa/Dar_es_Salaam"
                | "Africa/Kampala" | "Africa/Mogadishu"
                | "Africa/Djibouti" | "Indian/Antananarivo"
                | "Indian/Comoro" | "Indian/Mayotte" => "EAT",
                "Africa/Casablanca" => "WET",
                "Africa/Accra" | "Africa/Abidjan" | "Africa/Bamako"
                | "Africa/Banjul" | "Africa/Conakry" | "Africa/Dakar"
                | "Africa/Freetown" | "Africa/Lome" | "Africa/Monrovia"
                | "Africa/Nouakchott" | "Africa/Ouagadougou"
                | "Africa/Sao_Tome" => "GMT",
                "Africa/Tripoli" => "EET",
                "Africa/Khartoum" | "Africa/Juba" => "CAT",

                // Oceania
                "Australia/Sydney" | "Australia/Melbourne"
                | "Australia/Hobart" | "Australia/Currie" => "AEST",
                "Australia/Brisbane" | "Australia/Lindeman" => "AEST",
                "Australia/Adelaide" | "Australia/Broken_Hill" => "ACST",
                "Australia/Darwin" => "ACST",
                "Australia/Perth" => "AWST",
                "Australia/Lord_Howe" => "LHST",
                "Australia/Eucla" => "ACWST",
                "Pacific/Auckland" | "Pacific/Chatham" => "NZST",
                "Pacific/Fiji" => "FJT",
                "Pacific/Tongatapu" => "TOT",
                "Pacific/Apia" => "WST",
                "Pacific/Port_Moresby" => "PGT",
                "Pacific/Guam" | "Pacific/Palau" => "ChST",
                "Pacific/Noumea" => "NCT",
                "Pacific/Guadalcanal" => "SBT",
                "Pacific/Tarawa" | "Pacific/Majuro" | "Pacific/Kwajalein" => "MHT",
                "Pacific/Pago_Pago" | "Pacific/Midway" | "US/Samoa" => "SST",
                "Pacific/Gambier" => "GAMT",
                "Pacific/Marquesas" => "MART",
                "Pacific/Tahiti" => "TAHT",
                "Pacific/Pitcairn" => "PST",
                "Pacific/Easter" => "EAST",
                "Pacific/Galapagos" => "GALT",
                "Pacific/Norfolk" => "NFT",
                "Pacific/Efate" => "VUT",
                "Pacific/Kosrae" => "KOST",
                "Pacific/Pohnpei" | "Pacific/Ponape" => "PONT",
                "Pacific/Chuuk" | "Pacific/Truk" => "CHUT",
                "Pacific/Nauru" => "NRT",
                "Pacific/Funafuti" => "TVT",
                "Pacific/Wake" => "WAKT",
                "Pacific/Wallis" => "WFT",
                "Pacific/Kiritimati" => "LINT",

                // Indian Ocean
                "Indian/Maldives" => "MVT",
                "Indian/Mauritius" => "MUT",
                "Indian/Reunion" => "RET",
                "Indian/Chagos" => "IOT",
                "Indian/Christmas" => "CXT",
                "Indian/Cocos" => "CCT",
                "Indian/Kerguelen" => "TFT",
                "Indian/Mahe" => "SCT",

                _ => return None,
            };
            Some(abbr.to_string())
        })
        .unwrap_or_else(|| {
            iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string())
        })
}

pub fn utc_to_local(utc_str: &str) -> String {
    NaiveDateTime::parse_from_str(utc_str, "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|naive| Utc.from_local_datetime(&naive).single())
        .map(|utc_dt| {
            let local = utc_dt.with_timezone(&Local);
            format!("{} {}", local.format("%Y-%m-%d %H:%M"), timezone_abbr())
        })
        .unwrap_or_else(|| utc_str.to_string())
}
