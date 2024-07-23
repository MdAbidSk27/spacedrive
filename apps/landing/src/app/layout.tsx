import type { Metadata, Viewport } from 'next';
import { PropsWithChildren } from 'react';

import { Footer } from './Footer';
import { NavBar } from './NavBar';

import '@sd/ui/style/style.scss';
import '~/styles/prism.css';
import '~/styles/style.scss';

import PlausibleProvider from 'next-plausible';

import { Providers } from './Providers';

export const metadata: Metadata = {
	metadataBase: new URL('https://spacedrive.com'),
	robots: 'index, follow',
	description:
		'Combine your drives and clouds into one database that you can organize and explore from any device. Designed for creators, hoarders and the painfully disorganized.',
	openGraph: {
		images: 'https://spacedrive.com/logo.png'
	},
	keywords:
		'files,file manager,spacedrive,file explorer,vdfs,distributed filesystem,cas,content addressable storage,virtual filesystem,photos app, video organizer,video encoder,tags,tag based filesystem',
	authors: { name: 'Spacedrive Technology Inc.', url: 'https://spacedrive.com' }
};

export const viewport: Viewport = {
	themeColor: { color: '#E751ED', media: 'not screen' }
};

export default function Layout({ children }: PropsWithChildren) {
	return (
		<html lang="en" className="dark scroll-smooth">
			<head>
				<PlausibleProvider
					domain="spacedrive.com"
					customDomain="spacedrive.com"
					trackOutboundLinks
					taggedEvents
				/>
			</head>
			<body>
				<Providers>
					<div className="overflow-hidden dark:bg-[#030014]/60">
						<NavBar />
						<main className="dark z-10 m-auto max-w-[100rem] dark:text-white">
							{children}
						</main>
						<Footer />
					</div>
				</Providers>
			</body>
		</html>
	);
}
